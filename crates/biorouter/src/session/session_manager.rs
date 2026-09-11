use crate::config::paths::Paths;
use crate::config::ExtensionConfig;
use crate::conversation::message::{
    new_message_id, Message, MessageContent, MessageMetadata, TokenState,
};
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::privacy::SessionClassification;
use crate::providers::base::{Provider, MSG_COUNT_FOR_SESSION_NAME_GENERATION};
use crate::providers::pricing::{
    cost_with_pricing, provider_model_pricing, resolved_provider_model_pricing,
    ProviderModelPricing, TurnCost,
};
use crate::session::chat_fts;
use crate::session::extension_data::ExtensionData;
use crate::session::message_blobs;
use crate::workflow::Workflow;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rmcp::model::Role;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use tracing::{debug, info, warn};
use utoipa::ToSchema;

pub const CURRENT_SCHEMA_VERSION: i32 = 20;

/// The arm that first ran issue #56's classification backfill.
///
/// Named because the migration tests need "one below the backfill" and
/// `CURRENT_SCHEMA_VERSION - 1` stopped meaning that the moment arm 20 landed.
#[cfg(test)]
const PRIVACY_BACKFILL_ARM: i32 = 19;

/// The arm that re-runs the backfill with every evidence source, repairing the
/// rows [`PRIVACY_BACKFILL_ARM`] classified from the bound provider alone.
///
/// Named because [`SessionStorage::retreat_to_privacy_repair`] has to name the
/// number it rewinds to, and a literal there that drifted from the arm would
/// silently rewind to a version that re-runs nothing.
const PRIVACY_REPAIR_ARM: i32 = 20;
pub const SESSIONS_FOLDER: &str = "sessions";
pub const DB_NAME: &str = "sessions.db";

/// FTS5 mirror of user-visible message text, used for relevance-ranked chat
/// recall (BR-17). It is a contentful FTS5 table (it stores the flattened
/// text) maintained from Rust at the message write sites, because the searchable
/// text is a derived flattening of `content_json`, not a raw column, so SQLite
/// content-sync triggers can't produce it. `message_id`/`session_id` are stored
/// UNINDEXED so recall can join back to `messages`/`sessions` and delete a
/// session's rows on the compaction rewrite.
const MESSAGES_FTS_DDL: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    text,
    session_id UNINDEXED,
    message_id UNINDEXED,
    tokenize = 'porter unicode61'
)
"#;

const MESSAGES_FTS_INSERT: &str =
    "INSERT INTO messages_fts (text, session_id, message_id) VALUES (?, ?, ?)";

/// The append-only declassification ledger (issue #56, §12.5). A constant
/// because the two schema paths that create it run against different handles —
/// `create_schema` against the pool, the reconcile against the one connection
/// holding its write transaction.
const CLASSIFICATION_AUDIT_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS classification_audit (
  id                      INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id              TEXT NOT NULL,
  from_classification     TEXT NOT NULL,
  to_classification       TEXT NOT NULL,
  reason                  TEXT NOT NULL,
  actor                   TEXT NOT NULL,
  actor_kind              TEXT NOT NULL,
  occurred_at             TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  app_version             TEXT NOT NULL,
  provider_name_at_change TEXT,
  privacy_reason_before   TEXT,
  message_count_at_change INTEGER
)
"#;

/// The user's cross-affiliation grants (issue #56, DR-26 / Task 49): one row per
/// accepted (session, extension, model affiliation) triple. See
/// [`crate::privacy::grant`], which owns every read and the one write.
///
/// A constant for the same reason [`CLASSIFICATION_AUDIT_DDL`] is one — the two
/// schema paths that create it run against different handles, `create_schema`
/// against the pool and the reconcile against the connection holding its write
/// transaction.
///
/// ⚠ **No numbered migration arm, deliberately.** This table is additive and
/// version-independent, so it is created by `ensure_privacy_schema` on every
/// startup rather than by a new `apply_migration` number. Issue #56's own O10
/// hazard is a migration number consumed by testers: every developer's database —
/// and the operator's — already stands at `CURRENT_SCHEMA_VERSION` and would
/// never re-enter a newly written arm, so a table added there would not exist on
/// any machine that has opened this branch. `CREATE TABLE IF NOT EXISTS` in the
/// idempotent reconcile is the shape this tree already uses for the checkpoints
/// and message-blob tables.
///
/// The composite primary key is the triple, which is what makes re-approval an
/// upsert rather than a duplicate row.
const CROSS_AFFILIATION_GRANTS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS cross_affiliation_grants (
  session_id        TEXT NOT NULL,
  extension         TEXT NOT NULL,
  model_affiliation TEXT NOT NULL,
  granted_at        TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  app_version       TEXT NOT NULL,
  PRIMARY KEY (session_id, extension, model_affiliation)
)
"#;

/// Usage of deleted chats, folded out of `token_events` when a chat is deleted
/// (F10): one row per local calendar day, model and provider, holding sums and
/// the number of turns behind them — and nothing that identifies a chat: no
/// session id, no event key, no turn time. `''` stands for an unknown model or
/// provider, because a primary key cannot tell two NULLs apart.
///
/// Each `*_known` counts the turns that reported that bucket, and each sum
/// covers only those turns, so a total drawn from here is exactly as complete —
/// or as incomplete — as the turns it came from.
///
/// ⚠ **No numbered migration arm**, for the reason the grants table above gives:
/// it is additive, so `create_usage_schema` creates it for a fresh database and
/// `reconcile_usage_schema` for every existing one, on every open.
const DELETED_CHAT_USAGE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS deleted_chat_usage (
  day                   TEXT    NOT NULL,
  model_id              TEXT    NOT NULL DEFAULT '',
  provider              TEXT    NOT NULL DEFAULT '',
  turns                 INTEGER NOT NULL DEFAULT 0,
  input_tokens          INTEGER NOT NULL DEFAULT 0,
  input_known           INTEGER NOT NULL DEFAULT 0,
  output_tokens         INTEGER NOT NULL DEFAULT 0,
  output_known          INTEGER NOT NULL DEFAULT 0,
  billed_total_tokens   INTEGER NOT NULL DEFAULT 0,
  billed_known          INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
  cache_read_known      INTEGER NOT NULL DEFAULT 0,
  cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
  cache_creation_known  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, model_id, provider)
)
"#;

/// Billable usage: the relation the Usage panel, `biorouter usage` and the
/// global per-model rollup total (F10). Aliased `u`, with two halves of one
/// shape:
///
/// - every billable turn of a chat that still exists, one row per turn —
///   joined to `sessions`, so a per-turn row whose chat is gone is never read;
/// - [`DELETED_CHAT_USAGE_DDL`]'s rows, dated at their day's local midnight, so
///   a window that starts on a day boundary and ends now — the Usage panel's —
///   always contains them.
///
/// Deleted chats stay in these totals on purpose. The Usage panel exists so a
/// user can hold Biorouter's numbers against the provider's own meter and the
/// UCSF allowance (issue #1), and a delete does not refund a token there.
/// Home's heatmap and insight tiles count activity rather than spend and read
/// only chats that still exist — the same split that already counts subagent
/// spend here and not there.
///
/// `turns` stands in for `COUNT(*)` and each `*_known` for `COUNT(column)`, so a
/// total is complete only when every turn behind it reported that bucket — the
/// rule the per-turn queries applied before the fold existed.
const BILLABLE_USAGE: &str = "(
    SELECT te.ts AS ts,
           date(te.ts, 'unixepoch', 'localtime') AS day,
           te.model_id AS model_id,
           te.provider AS provider,
           1 AS turns,
           te.input_tokens AS input_tokens,
           te.input_tokens IS NOT NULL AS input_known,
           te.output_tokens AS output_tokens,
           te.output_tokens IS NOT NULL AS output_known,
           te.billed_total_tokens AS billed_total_tokens,
           te.billed_total_tokens IS NOT NULL AS billed_known,
           te.cache_read_tokens AS cache_read_tokens,
           te.cache_read_tokens IS NOT NULL AS cache_read_known,
           te.cache_creation_tokens AS cache_creation_tokens,
           te.cache_creation_tokens IS NOT NULL AS cache_creation_known
      FROM token_events te
      JOIN sessions s ON s.id = te.session_id
     WHERE te.session_type IN ('user', 'scheduled', 'sub_agent')
    UNION ALL
    SELECT CAST(strftime('%s', d.day, 'utc') AS INTEGER),
           d.day,
           NULLIF(d.model_id, ''),
           NULLIF(d.provider, ''),
           d.turns,
           d.input_tokens, d.input_known,
           d.output_tokens, d.output_known,
           d.billed_total_tokens, d.billed_known,
           d.cache_read_tokens, d.cache_read_known,
           d.cache_creation_tokens, d.cache_creation_known
      FROM deleted_chat_usage d
) u";

/// The `(day, model, provider)` grain every priced usage total starts from,
/// over [`BILLABLE_USAGE`]. `day` is the expression for the `day` column —
/// `u.day`, or `''` for a caller that sums across days — and `filter` the
/// `WHERE` clause. One builder, so the three priced totals cannot drift apart
/// in how they turn `turns` and `*_known` back into completeness.
fn billable_usage_grain_sql(day: &str, filter: &str) -> String {
    format!(
        "SELECT {day} AS day,
                u.model_id,
                u.provider,
                COALESCE(SUM(u.input_tokens), 0)  AS input_tokens,
                COALESCE(SUM(u.output_tokens), 0) AS output_tokens,
                CASE WHEN SUM(u.billed_known) = SUM(u.turns)
                     THEN SUM(u.billed_total_tokens) END AS total_tokens,
                CASE WHEN SUM(u.cache_read_known) = SUM(u.turns)
                     THEN SUM(u.cache_read_tokens) END AS cache_read_tokens,
                CASE WHEN SUM(u.cache_creation_known) = SUM(u.turns)
                     THEN SUM(u.cache_creation_tokens) END AS cache_creation_tokens,
                SUM(u.turns) AS turns,
                CAST(SUM(u.input_known) = SUM(u.turns) AS INTEGER) AS input_complete,
                CAST(SUM(u.output_known) = SUM(u.turns) AS INTEGER) AS output_complete,
                CAST(SUM(u.cache_read_known) = SUM(u.turns) AS INTEGER) AS cache_read_complete,
                CAST(SUM(u.cache_creation_known) = SUM(u.turns) AS INTEGER) AS cache_creation_complete
           FROM {BILLABLE_USAGE}
          WHERE {filter}
          GROUP BY 1, u.model_id, u.provider"
    )
}

/// True when `err` is the `UNIQUE(messages.session_id, messages.msg_uid)`
/// violation (SQLite error 2067) from the message insert — the one failure
/// [`SessionStorage::add_message`] recovers from by re-minting the uid (#41).
/// Scoped to the msg_uid index by message text so an unrelated unique
/// violation still surfaces as an error.
fn is_msg_uid_unique_violation(err: &anyhow::Error) -> bool {
    match err.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::Database(db_err)) => {
            db_err.is_unique_violation() && db_err.message().contains("messages.msg_uid")
        }
        _ => false,
    }
}

/// Whether a stored message should be indexed for recall. Recall searches what
/// the *user* saw, so only `user_visible` messages are indexed. A row with no
/// (or unparseable) metadata predates the flag and defaults to visible.
fn message_is_user_visible(metadata_json: Option<&str>) -> bool {
    metadata_json
        .and_then(|json| serde_json::from_str::<MessageMetadata>(json).ok())
        .map(|meta| meta.user_visible)
        .unwrap_or(true)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    #[default]
    User,
    Scheduled,
    SubAgent,
    Hidden,
    Terminal,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::User => write!(f, "user"),
            SessionType::SubAgent => write!(f, "sub_agent"),
            SessionType::Hidden => write!(f, "hidden"),
            SessionType::Scheduled => write!(f, "scheduled"),
            SessionType::Terminal => write!(f, "terminal"),
        }
    }
}

impl std::str::FromStr for SessionType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(SessionType::User),
            "sub_agent" => Ok(SessionType::SubAgent),
            "hidden" => Ok(SessionType::Hidden),
            "scheduled" => Ok(SessionType::Scheduled),
            "terminal" => Ok(SessionType::Terminal),
            _ => Err(anyhow::anyhow!("Invalid session type: {}", s)),
        }
    }
}

static SESSION_STORAGE: LazyLock<Arc<SessionStorage>> =
    LazyLock::new(|| Arc::new(SessionStorage::new(Paths::data_dir())));

pub const DEFAULT_SESSION_NAME: &str = "New chat";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: String,
    #[schema(value_type = String)]
    pub working_dir: PathBuf,
    #[serde(alias = "description")]
    pub name: String,
    #[serde(default)]
    pub user_set_name: bool,
    #[serde(default)]
    pub session_type: SessionType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub extension_data: ExtensionData,
    /// The *current* turn's usage — the live context-window occupancy. Bounded by
    /// the model's context limit, so `i32` is safe.
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    /// Lifetime totals. New usage writes use the four-bucket billed total;
    /// databases created before billed-bucket accounting may contain legacy
    /// context totals here, so reporting and budgets use `token_events` instead.
    /// These counters grow without bound, so SQLite and Rust both use 64-bit.
    pub accumulated_total_tokens: Option<i64>,
    pub accumulated_input_tokens: Option<i64>,
    pub accumulated_output_tokens: Option<i64>,
    pub schedule_id: Option<String>,
    pub workflow: Option<Workflow>,
    pub user_workflow_values: Option<HashMap<String, String>>,
    pub conversation: Option<Conversation>,
    pub message_count: usize,
    pub provider_name: Option<String>,
    pub model_config: Option<ModelConfig>,
    /// Id of the session this one was diverged (branched) from, if any. Set by
    /// `diverge_session`; `None` for normally-created sessions. Lets the UI show
    /// a session's lineage ("branched from …").
    #[serde(default)]
    pub diverged_from: Option<String>,
    /// The durable `msg_uid` of the exact parent message this session was
    /// branched at — the divergence point (BR-45). Paired with `diverged_from`
    /// (parent session), it is the edge label of the branch forest. `None` for
    /// normally-created sessions. Anchoring on this stable id instead of a
    /// whole-second timestamp is what fixes the same-second over-truncation.
    #[serde(default)]
    pub branch_point_msg_uid: Option<String>,
    /// Id of the parent session that spawned this one as a subagent (BR-71).
    /// Sibling of `diverged_from` (branch lineage): `diverged_from` records a
    /// user fork; this records a delegation. `None` for non-subagent sessions.
    /// It is what the interface groups tabs by, and what
    /// `refuse_unless_direct_subagent_child` reads to keep a delegation-scoped
    /// grant pointed at its own children. It is NOT read by the §7 capability
    /// matrix any more: that matrix had an `L` axis until lineage stopped being
    /// a boundary for writes (issue #56).
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// How sensitive this session's contents are (issue #56). A permanent
    /// ratchet: the storage layer's `CASE WHEN` refuses to lower it, and
    /// `privacy::declassify` is the only writer in the tree permitted to.
    #[serde(default = "SessionClassification::public")]
    pub privacy_tier: SessionClassification,
    /// Audit and UX only — never read by a gate. One of `turn:<provider>`,
    /// `mcp:<extension>`, `inherited:<parent_id>`, `diverged:<parent_id>`,
    /// `backfill:<provider>`, `declassified_by_user`. §12.4 grades the
    /// declassification confirmation on whether it has ever been `mcp:*`.
    ///
    /// It therefore holds the **dominant** provenance, not the first or the
    /// latest one: the storage layer lets an `mcp:` raise displace a non-`mcp:`
    /// reason and lets nothing displace an `mcp:` reason. Freezing it on the
    /// first raise would answer "has it ever been `mcp:*`" with a flat no, since
    /// Gate B's `turn:*` always lands before Gate C's `mcp:*`.
    #[serde(default)]
    pub privacy_reason: Option<String>,
}

// `sqlx::FromRow` is hand-written rather than derived (see below) so
// `privacy_tier` can use the same fail-closed read as [`Session`]. Deliberately
// not a doc comment: utoipa publishes those as the schema's `description`, and
// a note about a Rust row decoder is noise in the generated TypeScript client.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionSummary {
    pub id: String,
    pub working_dir: String,
    pub name: String,
    #[serde(default)]
    pub user_set_name: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: i64,
    /// BR-71: `sub_agent` rows are grouped under this parent in History.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// BR-71: the session's type as stored (`user`/`scheduled`/`sub_agent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_type: Option<String>,
    /// BR-45: the session this one was branched from, or `None`.
    ///
    /// Carried on the SUMMARY, not just on the full [`Session`], because the
    /// sidebar draws a glyph per chat kind and had no other way to know. Its
    /// only remaining signal was the default branch NAME (`"… (branch 2)"`),
    /// which anyone can rename away: of the branches on this machine, one in
    /// five had been renamed and so drew as an ordinary chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diverged_from: Option<String>,
    /// Issue #56. Carried here so the sidebar's recent-chats list can badge a
    /// private chat without an N+1 `get_session` per row.
    #[serde(default = "SessionClassification::public")]
    pub privacy_tier: SessionClassification,
}

/// One turn's token usage, applied additively and atomically in SQL.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenDelta {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub total: Option<i64>,
}

impl TokenDelta {
    fn is_empty(self) -> bool {
        self.input.is_none() && self.output.is_none() && self.total.is_none()
    }
}

/// One provider call's durable accounting payload. `event_key` identifies the
/// provider call for idempotent retries: the ledger row and the session's
/// accumulated counters are either both applied once or neither is applied.
#[derive(Debug, Clone)]
pub struct UsageLedgerEntry {
    pub event_key: String,
    pub session_id: String,
    pub schedule_id: Option<String>,
    pub current_total_tokens: Option<i32>,
    pub current_input_tokens: Option<i32>,
    pub current_output_tokens: Option<i32>,
    pub billed_total_tokens: Option<i64>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub model_id: Option<String>,
    pub provider: Option<String>,
    pub cache_read_tokens: Option<i32>,
    pub cache_creation_tokens: Option<i32>,
}

pub struct SessionUpdateBuilder<'a> {
    session_manager: &'a SessionManager,
    session_id: String,
    name: Option<String>,
    user_set_name: Option<bool>,
    session_type: Option<SessionType>,
    working_dir: Option<PathBuf>,
    extension_data: Option<ExtensionData>,
    total_tokens: Option<Option<i32>>,
    input_tokens: Option<Option<i32>>,
    output_tokens: Option<Option<i32>>,
    accumulated_total_tokens: Option<Option<i64>>,
    accumulated_input_tokens: Option<Option<i64>>,
    accumulated_output_tokens: Option<Option<i64>>,
    /// A per-turn DELTA, applied atomically as `col = COALESCE(col,0) + ?` in
    /// SQL. The old path read the row into Rust, added, and wrote back — a
    /// lost-update race whenever two turns raced on one session.
    token_delta: Option<TokenDelta>,
    schedule_id: Option<Option<String>>,
    workflow: Option<Option<Workflow>>,
    user_workflow_values: Option<Option<HashMap<String, String>>>,
    provider_name: Option<Option<String>>,
    model_config: Option<Option<ModelConfig>>,
    diverged_from: Option<Option<String>>,
    branch_point_msg_uid: Option<Option<String>>,
    parent_session_id: Option<Option<String>>,
    /// Raise-only. There is deliberately NO setter that accepts an arbitrary
    /// value, and the SQL refuses a lowering write even if one appeared.
    privacy_raise: Option<(SessionClassification, String)>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionInsights {
    pub total_sessions: usize,
    pub total_tokens: Option<i64>,
    pub sessions_last_7_days: usize,
    pub sessions_last_30_days: usize,
    pub tokens_last_7_days: Option<i64>,
    pub tokens_last_30_days: Option<i64>,
}

/// The session types a user actually sees. `SubAgent`, `Hidden` and `Terminal`
/// sessions are internal machinery — one user task can spawn several — so
/// counting them made the insight tiles disagree with the session list printed
/// directly beneath them. This mirrors what `list_sessions` shows.
pub const USER_FACING_SESSION_TYPES: [&str; 2] = ["user", "scheduled"];

/// One calendar day of usage, for the Home heatmap.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    /// Local calendar day, `YYYY-MM-DD`.
    pub date: String,
    /// Sessions *started* that day (exact; keyed on the immutable `created_at`).
    pub sessions: i64,
    /// Tokens processed that day, summed from per-turn `token_events`.
    pub tokens: i64,
    /// False when at least one token event that day lacks billed-token
    /// accounting. `tokens` is then a known subtotal; zero is unavailable, not
    /// a measured zero.
    pub tokens_complete: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Assistant + user messages exchanged that day.
    pub messages: i64,
    /// 1–4. Level 0 days are omitted from the response entirely.
    pub level: u8,
}

/// The Home heatmap payload.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivityWindow {
    pub start: String,
    pub end: String,
    pub max_sessions: i64,
    pub max_tokens: i64,
    /// False when at least one event in the window lacks billed-token accounting.
    /// Consult each day's `tokens_complete` for display semantics.
    pub tokens_complete: bool,
    pub current_streak: i64,
    pub longest_streak: i64,
    /// Only days with activity. The client fills the rest of the grid with level 0.
    pub days: Vec<DailyActivity>,
}

/// One `(model, provider)` group of the per-model usage breakdown.
///
/// `model_id` / `provider` are `None` for turns recorded before model
/// attribution landed, or when the provider reported no model — those rows
/// aggregate together as the "unknown" group.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageRow {
    pub model_id: Option<String>,
    pub provider: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Billed tokens across all four disjoint buckets. `None` means at least
    /// one contributing event has no reconstructable billed total.
    pub total_tokens: Option<i64>,
    /// Input tokens served from the prompt cache. `None` means at least one
    /// contributing event predates cache accounting or did not report it.
    pub cache_read_tokens: Option<i64>,
    /// Input tokens written to the prompt cache.
    pub cache_creation_tokens: Option<i64>,
    /// Number of billed turns attributed to this group.
    pub turns: i64,
}

/// How `get_usage_report` buckets the per-turn ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageGroup {
    /// One row per local calendar day (models summed within the day).
    Day,
    /// One row per `(model, provider)` group over the whole range.
    Model,
    /// One row per `(day, model, provider)`.
    DayModel,
}

/// One bucket of the usage report.
///
/// `date` is present unless grouping by [`UsageGroup::Model`]; `modelId` is
/// present unless grouping by [`UsageGroup::Day`]. `cost` is the dollar cost of
/// the priced turns in the bucket, or `None` when *every* contributing turn was
/// unpriced (an unknown model) — a `null` cost never means "$0". `hasUnpriced`
/// flags a bucket that mixes priced, unpriced, or incomplete turns, so a day
/// cost can be read as "at least this much" rather than exact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageReportRow {
    pub date: Option<String>,
    pub model_id: Option<String>,
    pub provider: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Billed tokens, or `None` when the bucket includes incomplete history.
    pub total_tokens: Option<i64>,
    /// Prompt-cache read/creation tokens in the bucket. `None` preserves
    /// historical incompleteness; it must not be presented as a measured zero.
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub turns: i64,
    pub cost: Option<f64>,
    pub has_unpriced: bool,
    /// `true` when cache cost is omitted because a contributing model has no
    /// cache rate or an event did not report a required cache bucket.
    pub cost_excludes_cache: bool,
}

/// Token + cost totals for a time span, priced through [`model_cost_with_cache`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Billed tokens, or `None` when the span includes incomplete history.
    pub total_tokens: Option<i64>,
    /// Prompt-cache read/creation tokens in the span. `None` means the span
    /// includes at least one event without cache-bucket accounting.
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub turns: i64,
    /// Dollar cost of the priced turns, or `None` when nothing in the span is
    /// priced. Priced-but-partial spans return the priced sum with
    /// `has_unpriced = true`.
    pub cost: Option<f64>,
    pub has_unpriced: bool,
    /// `true` when cache cost is omitted because pricing or cache accounting is
    /// incomplete — the figure is then a lower bound.
    pub cost_excludes_cache: bool,
}

/// Month-to-date and all-time usage totals, for the summary gauge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    /// Current local month, `YYYY-MM`.
    pub month: String,
    pub month_to_date: UsageTotals,
    pub all_time: UsageTotals,
}

/// The finest per-`(day, model, provider)` grain the report SQL returns, before
/// Rust rolls it up into the requested [`UsageGroup`] and prices each bucket.
#[derive(Debug, Clone, sqlx::FromRow)]
struct UsageGrainRow {
    day: String,
    model_id: Option<String>,
    provider: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    turns: i64,
    input_complete: i64,
    output_complete: i64,
    cache_read_complete: i64,
    cache_creation_complete: i64,
}

/// Cost of one finest-grain row, including its cache buckets, or `None` when its
/// `(provider, model)` pair is unknown/unpriced. Both endpoints must be present
/// to price it — a row with no model (the "unknown" bucket) is always unpriced.
struct GrainPrice {
    cost: Option<TurnCost>,
    incomplete: bool,
}

type ResolvedPricing = HashMap<(String, String), ProviderModelPricing>;

async fn resolve_grain_pricing(grain: &[UsageGrainRow]) -> ResolvedPricing {
    let mut resolved = HashMap::new();
    for row in grain {
        let (Some(provider), Some(model)) = (row.provider.as_ref(), row.model_id.as_ref()) else {
            continue;
        };
        let key = (provider.to_ascii_lowercase(), model.to_ascii_lowercase());
        if resolved.contains_key(&key) {
            continue;
        }
        if let Some(pricing) = resolved_provider_model_pricing(provider, model).await {
            resolved.insert(key, pricing);
        }
    }
    resolved
}

fn price_grain(row: &UsageGrainRow, resolved: &ResolvedPricing) -> GrainPrice {
    let incomplete_input_output = row.input_complete == 0 || row.output_complete == 0;
    let (Some(provider), Some(model)) = (row.provider.as_deref(), row.model_id.as_deref()) else {
        return GrainPrice {
            cost: None,
            incomplete: true,
        };
    };
    let key = (provider.to_ascii_lowercase(), model.to_ascii_lowercase());
    let pricing = resolved
        .get(&key)
        .cloned()
        .or_else(|| provider_model_pricing(provider, model));
    let Some(pricing) = pricing else {
        return GrainPrice {
            cost: None,
            incomplete: true,
        };
    };
    let incomplete_cache = row.cache_read_complete == 0 || row.cache_creation_complete == 0;
    let incomplete = incomplete_input_output || incomplete_cache;

    let cost = Some(cost_with_pricing(
        &pricing,
        row.input_tokens,
        row.output_tokens,
        row.cache_read_tokens.unwrap_or(0),
        row.cache_creation_tokens.unwrap_or(0),
    ))
    .map(|mut cost| {
        cost.cache_excluded |= incomplete_cache;
        cost
    });

    // A known model with only a context total has no priceable token buckets.
    // Returning Some(0) would falsely turn "unknown" into "$0".
    let cost = match cost {
        Some(cost) if incomplete && cost.cost == 0.0 && row.total_tokens != Some(0) => None,
        other => other,
    };

    GrainPrice { cost, incomplete }
}

/// Accumulator for one output bucket while rolling grain rows up.
#[derive(Default)]
struct BucketAcc {
    date: Option<String>,
    model_id: Option<String>,
    provider: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    turns: i64,
    cost_sum: f64,
    priced_any: bool,
    unpriced_any: bool,
    /// Set when a priced grain row carried cache tokens the model has no rate
    /// for, so `cost_sum` understates the true figure.
    cache_excluded_any: bool,
}

impl BucketAcc {
    fn add(&mut self, row: &UsageGrainRow, resolved: &ResolvedPricing) {
        let first_row = self.turns == 0;
        self.input_tokens += row.input_tokens;
        self.output_tokens += row.output_tokens;
        self.total_tokens = if first_row {
            row.total_tokens
        } else {
            sum_complete(self.total_tokens, row.total_tokens)
        };
        self.cache_read_tokens = if first_row {
            row.cache_read_tokens
        } else {
            sum_complete(self.cache_read_tokens, row.cache_read_tokens)
        };
        self.cache_creation_tokens = if first_row {
            row.cache_creation_tokens
        } else {
            sum_complete(self.cache_creation_tokens, row.cache_creation_tokens)
        };
        self.turns += row.turns;
        let price = price_grain(row, resolved);
        self.unpriced_any |= price.incomplete;
        match price.cost {
            Some(TurnCost {
                cost,
                cache_excluded,
            }) => {
                self.cost_sum += cost;
                self.priced_any = true;
                self.cache_excluded_any |= cache_excluded;
            }
            None => self.unpriced_any = true,
        }
    }

    /// `cost` is `None` only when the bucket is *entirely* unpriced, so a null
    /// cost can never be misread as "$0"; a partially-priced bucket returns its
    /// priced sum with `has_unpriced = true`.
    fn cost(&self) -> Option<f64> {
        (self.priced_any && !(self.unpriced_any && self.cost_sum == 0.0)).then_some(self.cost_sum)
    }
}

fn sum_complete(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left + right),
        _ => None,
    }
}

fn complete_sum(count: i64, known_count: i64, sum: Option<i64>) -> Option<i64> {
    if count == 0 {
        Some(0)
    } else if count == known_count {
        sum
    } else {
        None
    }
}

/// Roll the finest per-`(day, model, provider)` grain up into the requested
/// grouping, pricing every grain row through [`model_cost_with_cache`] once. Pure so the
/// grouping + cost math is unit-tested without a database.
#[cfg(test)]
fn rollup_report(grain: &[UsageGrainRow], group: UsageGroup) -> Vec<UsageReportRow> {
    rollup_report_with_pricing(grain, group, &ResolvedPricing::new())
}

fn rollup_report_with_pricing(
    grain: &[UsageGrainRow],
    group: UsageGroup,
    resolved: &ResolvedPricing,
) -> Vec<UsageReportRow> {
    // Bucket key: (date, model_id, provider); components are None per grouping.
    type BucketKey = (Option<String>, Option<String>, Option<String>);
    let mut map: std::collections::BTreeMap<BucketKey, BucketAcc> =
        std::collections::BTreeMap::new();

    for row in grain {
        let (date, model_id, provider) = match group {
            UsageGroup::Day => (Some(row.day.clone()), None, None),
            UsageGroup::Model => (None, row.model_id.clone(), row.provider.clone()),
            UsageGroup::DayModel => (
                Some(row.day.clone()),
                row.model_id.clone(),
                row.provider.clone(),
            ),
        };
        map.entry((date.clone(), model_id.clone(), provider.clone()))
            .or_insert_with(|| BucketAcc {
                date,
                model_id,
                provider,
                ..Default::default()
            })
            .add(row, resolved);
    }

    let mut rows: Vec<UsageReportRow> = map
        .into_values()
        .map(|a| UsageReportRow {
            cost: a.cost(),
            has_unpriced: a.unpriced_any,
            cost_excludes_cache: a.cache_excluded_any,
            date: a.date,
            model_id: a.model_id,
            provider: a.provider,
            input_tokens: a.input_tokens,
            output_tokens: a.output_tokens,
            total_tokens: a.total_tokens,
            cache_read_tokens: a.cache_read_tokens,
            cache_creation_tokens: a.cache_creation_tokens,
            turns: a.turns,
        })
        .collect();

    // Day-bearing groups read as a chronological series (day asc, then heaviest
    // model first within a day); a pure per-model report is heaviest-first.
    rows.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(b.total_tokens.cmp(&a.total_tokens))
            .then(a.model_id.cmp(&b.model_id))
    });
    rows
}

/// Sum grain rows into a single priced total (used for MTD and all-time).
#[cfg(test)]
fn totals_from_grain(grain: &[UsageGrainRow]) -> UsageTotals {
    totals_from_grain_with_pricing(grain, &ResolvedPricing::new())
}

fn totals_from_grain_with_pricing(
    grain: &[UsageGrainRow],
    resolved: &ResolvedPricing,
) -> UsageTotals {
    if grain.is_empty() {
        return UsageTotals {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 0,
            cost: Some(0.0),
            has_unpriced: false,
            cost_excludes_cache: false,
        };
    }

    let mut acc = BucketAcc::default();
    for row in grain {
        acc.add(row, resolved);
    }
    UsageTotals {
        input_tokens: acc.input_tokens,
        output_tokens: acc.output_tokens,
        total_tokens: acc.total_tokens,
        cache_read_tokens: acc.cache_read_tokens,
        cache_creation_tokens: acc.cache_creation_tokens,
        turns: acc.turns,
        cost: acc.cost(),
        has_unpriced: acc.unpriced_any,
        cost_excludes_cache: acc.cache_excluded_any,
    }
}

/// A day's raw intensity, before bucketing.
///
/// Tokens lead; sessions break ties, so a day of deep work outranks a day of
/// three trivial sessions. Both are log-compressed because token counts are
/// heavy-tailed — one marathon day can be 30x a normal one, and on a linear
/// scale it flattens every other day to the faintest shade.
fn activity_score(sessions: i64, tokens: i64) -> f64 {
    if sessions == 0 && tokens == 0 {
        return 0.0;
    }
    (1.0 + tokens as f64).ln() + 0.5 * (1.0 + sessions as f64).ln()
}

/// Linear-interpolated quantile of a pre-sorted slice.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = (sorted.len() - 1) as f64 * q;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

/// Merge the three per-day queries into the heatmap payload.
///
/// Bucketing uses the **quartiles of the active days in this window**, not the
/// window maximum. Dividing by the max saturates: `ln` compresses so hard that
/// nearly every active day lands at 0.75-1.0 of the maximum, so a 4k-token day
/// renders as dark as a 250k-token day and the faintest level goes unused.
/// Quartiles give an even spread and match the GitHub convention every user
/// already recognises. Absolute values live in the tooltip, where they belong.
fn build_activity_window(
    start: String,
    end: String,
    session_rows: &[(String, i64)],
    token_rows: &[(String, i64, i64, i64, bool)],
    message_rows: &[(String, i64)],
) -> ActivityWindow {
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Day {
        sessions: i64,
        tokens: i64,
        input: i64,
        output: i64,
        messages: i64,
        has_incomplete_tokens: bool,
    }

    let mut by_day: BTreeMap<String, Day> = BTreeMap::new();

    for (day, n) in session_rows {
        by_day.entry(day.clone()).or_default().sessions += n;
    }
    for (day, tokens, input, output, tokens_complete) in token_rows {
        let d = by_day.entry(day.clone()).or_default();
        d.tokens += tokens;
        d.input += input;
        d.output += output;
        d.has_incomplete_tokens |= !tokens_complete;
    }
    for (day, n) in message_rows {
        by_day.entry(day.clone()).or_default().messages += n;
    }

    // A day is "active" if it started a session or spent a token. Messages alone
    // (an edited transcript, say) do not light a cell.
    let mut scores: Vec<f64> = by_day
        .values()
        .map(|d| activity_score(d.sessions, d.tokens))
        .filter(|s| *s > 0.0)
        .collect();
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let (q1, q2, q3) = (
        quantile(&scores, 0.25),
        quantile(&scores, 0.50),
        quantile(&scores, 0.75),
    );

    let max_sessions = by_day.values().map(|d| d.sessions).max().unwrap_or(0);
    let max_tokens = by_day.values().map(|d| d.tokens).max().unwrap_or(0);

    let days: Vec<DailyActivity> = by_day
        .iter()
        .filter_map(|(date, d)| {
            let score = activity_score(d.sessions, d.tokens);
            if score <= 0.0 {
                return None;
            }
            let level = if score <= q1 {
                1
            } else if score <= q2 {
                2
            } else if score <= q3 {
                3
            } else {
                4
            };
            Some(DailyActivity {
                date: date.clone(),
                sessions: d.sessions,
                tokens: d.tokens,
                tokens_complete: !d.has_incomplete_tokens,
                input_tokens: d.input,
                output_tokens: d.output,
                messages: d.messages,
                level,
            })
        })
        .collect();

    let (current_streak, longest_streak) = streaks(&start, &end, &days);

    ActivityWindow {
        start,
        end,
        max_sessions,
        max_tokens,
        tokens_complete: token_rows.iter().all(|row| row.4),
        current_streak,
        longest_streak,
        days,
    }
}

/// Consecutive active calendar days. `current` counts back from `end`.
fn streaks(start: &str, end: &str, days: &[DailyActivity]) -> (i64, i64) {
    use chrono::NaiveDate;
    use std::collections::HashSet;

    let active: HashSet<&str> = days.iter().map(|d| d.date.as_str()).collect();
    let (Ok(from), Ok(to)) = (
        NaiveDate::parse_from_str(start, "%Y-%m-%d"),
        NaiveDate::parse_from_str(end, "%Y-%m-%d"),
    ) else {
        return (0, 0);
    };

    let (mut longest, mut run) = (0i64, 0i64);
    let mut day = from;
    while day <= to {
        if active.contains(day.format("%Y-%m-%d").to_string().as_str()) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
        day = day.succ_opt().unwrap_or(day + chrono::Duration::days(1));
    }

    // The current streak may legitimately end yesterday: a user who has not yet
    // opened the app today has not broken it.
    let mut current = 0i64;
    let mut cursor = to;
    if !active.contains(cursor.format("%Y-%m-%d").to_string().as_str()) {
        cursor = cursor.pred_opt().unwrap_or(cursor);
    }
    while cursor >= from && active.contains(cursor.format("%Y-%m-%d").to_string().as_str()) {
        current += 1;
        let Some(prev) = cursor.pred_opt() else { break };
        cursor = prev;
    }

    (current, longest)
}

impl<'a> SessionUpdateBuilder<'a> {
    fn new(session_manager: &'a SessionManager, session_id: String) -> Self {
        Self {
            session_manager,
            session_id,
            name: None,
            user_set_name: None,
            session_type: None,
            working_dir: None,
            extension_data: None,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            token_delta: None,
            schedule_id: None,
            workflow: None,
            user_workflow_values: None,
            provider_name: None,
            model_config: None,
            diverged_from: None,
            branch_point_msg_uid: None,
            parent_session_id: None,
            privacy_raise: None,
        }
    }

    pub async fn apply(self) -> Result<()> {
        self.session_manager.apply_update_inner(self).await
    }

    pub fn user_provided_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.name = Some(name);
            self.user_set_name = Some(true);
        }
        self
    }

    pub fn system_generated_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.name = Some(name);
            self.user_set_name = Some(false);
        }
        self
    }

    pub fn session_type(mut self, session_type: SessionType) -> Self {
        self.session_type = Some(session_type);
        self
    }

    // NOTE: there is deliberately no `working_dir` setter here. The working
    // directory is guarded (#44): go through
    // `SessionManager::try_update_working_dir_if_empty`, or — for the terminal
    // shell-following path only — `force_update_working_dir_unguarded`.

    pub fn extension_data(mut self, data: ExtensionData) -> Self {
        self.extension_data = Some(data);
        self
    }

    pub fn total_tokens(mut self, tokens: Option<i32>) -> Self {
        self.total_tokens = Some(tokens);
        self
    }

    pub fn input_tokens(mut self, tokens: Option<i32>) -> Self {
        self.input_tokens = Some(tokens);
        self
    }

    pub fn output_tokens(mut self, tokens: Option<i32>) -> Self {
        self.output_tokens = Some(tokens);
        self
    }

    pub fn accumulated_total_tokens(mut self, tokens: Option<i64>) -> Self {
        // A column may appear only once in a SET list.
        self.token_delta = None;
        self.accumulated_total_tokens = Some(tokens);
        self
    }

    pub fn accumulated_input_tokens(mut self, tokens: Option<i64>) -> Self {
        // A column may appear only once in a SET list.
        self.token_delta = None;
        self.accumulated_input_tokens = Some(tokens);
        self
    }

    pub fn accumulated_output_tokens(mut self, tokens: Option<i64>) -> Self {
        // A column may appear only once in a SET list.
        self.token_delta = None;
        self.accumulated_output_tokens = Some(tokens);
        self
    }

    /// Add one turn's usage to the session's lifetime counters, atomically.
    ///
    /// Prefer this over the absolute `accumulated_*` setters for anything on the
    /// hot path: it compiles to `col = COALESCE(col, 0) + ?` so two concurrent
    /// turns on the same session cannot lose an update, and the arithmetic
    /// happens in SQLite's 64-bit INTEGER rather than in `i32`.
    pub fn accumulate_tokens(mut self, delta: TokenDelta) -> Self {
        if !delta.is_empty() {
            // A column may appear only once in a SET list.
            self.accumulated_total_tokens = None;
            self.accumulated_input_tokens = None;
            self.accumulated_output_tokens = None;
            self.token_delta = Some(delta);
        }
        self
    }

    pub fn schedule_id(mut self, schedule_id: Option<String>) -> Self {
        self.schedule_id = Some(schedule_id);
        self
    }

    pub fn workflow(mut self, workflow: Option<Workflow>) -> Self {
        self.workflow = Some(workflow);
        self
    }

    pub fn user_workflow_values(
        mut self,
        user_workflow_values: Option<HashMap<String, String>>,
    ) -> Self {
        self.user_workflow_values = Some(user_workflow_values);
        self
    }

    pub fn provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = Some(Some(provider_name.into()));
        self
    }

    pub fn model_config(mut self, model_config: ModelConfig) -> Self {
        self.model_config = Some(Some(model_config));
        self
    }

    /// Record (or clear) the id of the session this one was diverged from.
    pub fn diverged_from(mut self, diverged_from: Option<String>) -> Self {
        self.diverged_from = Some(diverged_from);
        self
    }

    /// Record (or clear) the durable `msg_uid` of the parent message this
    /// session was branched at (BR-45 divergence point).
    pub fn branch_point_msg_uid(mut self, branch_point_msg_uid: Option<String>) -> Self {
        self.branch_point_msg_uid = Some(branch_point_msg_uid);
        self
    }

    /// Record (or clear) the id of the session that spawned this one as a
    /// subagent (BR-71 delegation lineage).
    pub fn parent_session_id(mut self, parent_session_id: Option<String>) -> Self {
        self.parent_session_id = Some(parent_session_id);
        self
    }

    /// Raise the classification and record why. Monotone: passing `Public` to a
    /// row that is not exactly `public` is a no-op in SQL, so no caller — a
    /// route handler, a CLI command, a test, a future BR-71 tool, a hand-written
    /// query through this builder — can lower the tier.
    ///
    /// `reason` is likewise monotone, by dominance rather than by recency: an
    /// `mcp:` reason displaces a non-`mcp:` one and nothing displaces an `mcp:`
    /// one, so §12.4 can ask whether a private data source was ever reached.
    pub fn raise_privacy(mut self, to: SessionClassification, reason: &str) -> Self {
        self.privacy_raise = Some((to, reason.to_string()));
        self
    }
}

/// The six token counters stored on a session row. Fetched cheaply on the
/// streaming hot path without the surrounding metadata or message count.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct SessionTokenCounts {
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub accumulated_total_tokens: Option<i64>,
    pub accumulated_input_tokens: Option<i64>,
    pub accumulated_output_tokens: Option<i64>,
}

/// BR-52: the one place a session's stored counters become the `TokenState` the
/// clients see. The agent reads it once per turn boundary and carries it in the
/// event stream; the server no longer re-derives it per streamed token.
impl From<SessionTokenCounts> for TokenState {
    fn from(counts: SessionTokenCounts) -> Self {
        TokenState {
            input_tokens: counts.input_tokens.unwrap_or(0),
            output_tokens: counts.output_tokens.unwrap_or(0),
            total_tokens: counts.total_tokens.unwrap_or(0),
            accumulated_input_tokens: counts.accumulated_input_tokens.unwrap_or(0),
            accumulated_output_tokens: counts.accumulated_output_tokens.unwrap_or(0),
            accumulated_total_tokens: counts.accumulated_total_tokens.unwrap_or(0),
        }
    }
}

impl From<&Session> for TokenState {
    fn from(session: &Session) -> Self {
        TokenState {
            input_tokens: session.input_tokens.unwrap_or(0),
            output_tokens: session.output_tokens.unwrap_or(0),
            total_tokens: session.total_tokens.unwrap_or(0),
            accumulated_input_tokens: session.accumulated_input_tokens.unwrap_or(0),
            accumulated_output_tokens: session.accumulated_output_tokens.unwrap_or(0),
            accumulated_total_tokens: session.accumulated_total_tokens.unwrap_or(0),
        }
    }
}

/// SQLite row shape for the BR-43 `checkpoints` table, mapped to the public
/// `checkpoint::CheckpointRecord`.
#[derive(sqlx::FromRow)]
struct CheckpointRow {
    id: String,
    session_id: String,
    turn_index: i64,
    anchor_ts: i64,
    kind: String,
    commit_sha: String,
    tree_sha: String,
    changed_paths_json: String,
    created_at: String,
}

impl CheckpointRow {
    fn into_record(self) -> Result<crate::checkpoint::CheckpointRecord> {
        Ok(crate::checkpoint::CheckpointRecord {
            id: self.id,
            session_id: self.session_id,
            turn_index: self.turn_index,
            anchor_ts: self.anchor_ts,
            kind: self.kind.parse()?,
            commit_sha: self.commit_sha,
            tree_sha: self.tree_sha,
            changed_paths: serde_json::from_str(&self.changed_paths_json).unwrap_or_default(),
            created_at: self.created_at,
        })
    }
}

/// A monotone revision marker for one session's stored message set.
///
/// `messages.id` is `INTEGER PRIMARY KEY AUTOINCREMENT`, so SQLite keeps a
/// high-water mark in `sqlite_sequence` and never reuses a rowid. Every append
/// raises `max_rowid`. [`SessionManager::replace_conversation`] DELETEs and
/// re-INSERTs the whole set, so a rewrite raises it too — even when the content
/// is byte-identical. A delete lowers `count` and frees rowids that are never
/// minted again. The pair therefore cannot ABA, which a message count alone
/// demonstrably can: an edit that drops one message plus the next turn's user
/// message nets to zero.
///
/// Read through the `idx_messages_session` covering index, so it is index-only:
/// no table rows, no JSON. Cheaper than the `COUNT(*)` `get_session` already
/// does.
///
/// `(count, max_rowid)` is only non-repeating within ONE INCARNATION of the
/// session row, which is why `incarnation` is part of the token. A session id
/// is REUSABLE: `create_session` allocates `YYYYMMDD_N` as `MAX(N) + 1` over
/// the `sessions` table, so once that table is emptied the ids restart at 1.
/// Pair that with a rewound message sequence and a one-message session at
/// `(1, 1)` is reproducible by an entirely different conversation — a
/// detached rewrite from the previous incarnation would then pass the prefix
/// check and overwrite it (#51 W3). `incarnation` is minted per session ROW
/// from `random()` and never reused, so a basis taken before a wipe can never
/// match after one, whatever the rowids do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationRevision {
    /// Identity of the session ROW this revision was read from. `0` means
    /// "unknown" — a row written before the column existed and somehow missed
    /// the backfill. Two unknowns compare equal, degrading to the rowid guard
    /// alone rather than refusing every rewrite on such a database.
    incarnation: i64,
    count: i64,
    max_rowid: i64,
}

impl ConversationRevision {
    /// How many messages the session held at this revision.
    pub fn message_count(&self) -> usize {
        self.count.max(0) as usize
    }

    #[cfg(test)]
    pub(crate) fn from_parts(incarnation: i64, count: i64, max_rowid: i64) -> Self {
        Self {
            incarnation,
            count,
            max_rowid,
        }
    }
}

/// Outcome of [`SessionManager::replace_conversation_preserving_tail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceOutcome {
    /// Nothing landed since the basis; the replacement was stored verbatim.
    Replaced,
    /// Messages the caller had never seen landed since the basis. They were
    /// carried over onto the tail of the replacement instead of being deleted.
    ReplacedPreservingTail { preserved: usize },
    /// The basis itself was truncated or wholesale-rewritten underneath us, so
    /// there is no sound prefix to merge onto. NOTHING was written.
    Stale,
    /// No session with that id. NOTHING was written.
    SessionNotFound,
}

impl ReplaceOutcome {
    /// Did the rewrite actually land? `false` means the store is untouched.
    pub fn stored(&self) -> bool {
        matches!(
            self,
            ReplaceOutcome::Replaced | ReplaceOutcome::ReplacedPreservingTail { .. }
        )
    }
}

/// Outcome of [`SessionManager::truncate_conversation_bounded`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncateOutcome {
    /// The cut landed; `removed` message rows went with it.
    Truncated { removed: usize },
    /// The basis came from a previous incarnation of this session id, so its
    /// rowid watermark describes a conversation that no longer exists. NOTHING
    /// was deleted.
    Stale,
    /// No session with that id. NOTHING was deleted.
    SessionNotFound,
}

/// Outcome of [`SessionManager::try_update_working_dir_if_empty`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingDirUpdate {
    /// The session had no messages; the working dir was updated.
    Updated,
    /// The session already has at least one message; the working dir is fixed
    /// (#44) and was left untouched.
    RefusedNotEmpty,
    /// No session with that id exists; nothing was written.
    SessionNotFound,
}

pub struct SessionManager {
    storage: Arc<SessionStorage>,
}

impl SessionManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            storage: Arc::new(SessionStorage::new(data_dir)),
        }
    }

    pub fn instance() -> Self {
        Self {
            storage: Arc::clone(&SESSION_STORAGE),
        }
    }

    pub fn storage(&self) -> &Arc<SessionStorage> {
        &self.storage
    }

    pub async fn create_session(
        &self,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
    ) -> Result<Session> {
        self.storage
            .create_session(working_dir, name, session_type)
            .await
    }

    /// Record that this chat has reached an extension belonging to
    /// `institution` (issue #56, DR-26 / Task 50 Step 3).
    ///
    /// Monotone, and the union is computed **in SQL** for the reason
    /// [`SessionUpdateBuilder::raise_privacy`]'s `CASE WHEN` is: a
    /// read-modify-write from Rust has a window between the read and the write
    /// that a concurrent tool call fits inside, and the institution it recorded
    /// would be silently dropped. Two extensions of two institutions really can
    /// be dispatched in one parallel tool batch, so that window is not
    /// theoretical here.
    ///
    /// `NULL` and `''` are the empty set — every row that predates this column.
    /// A value that is not a JSON array is left alone rather than replaced:
    /// `json_each` errors on it and the statement fails, which surfaces as a
    /// refused turn (the caller `?`s) instead of silently erasing whatever was
    /// there.
    ///
    /// ⚠ **Known residual: "the extensions it touched" is the whole trigger.**
    /// Content can enter a chat from elsewhere — a `kb_search` against an
    /// institution's knowledge base, a `chatrecall` LOAD of that institution's
    /// chat — and neither records anything here, so the reading chat holds the
    /// content without carrying its owner. That is Step 3's literal scope, and
    /// the **tier** axis has had exactly the same boundary since Task 10:
    /// `raise_session_privacy` fires from the same one place, so reading a
    /// private base does not privatise the reading chat either. Widening it is
    /// one change to both axes. The symmetry is pinned by
    /// `extension_manager::tests::the_two_chat_side_ratchets_share_one_production_call_site`,
    /// which fails the build if the tier trigger widens and this one does not.
    pub async fn record_session_affiliation(
        &self,
        session_id: &str,
        institution: crate::privacy::affiliation::InstitutionId,
    ) -> Result<()> {
        self.storage
            .record_session_affiliation(session_id, institution.as_str())
            .await
    }

    /// The institutions whose extensions this chat has touched.
    ///
    /// ⚠ **`Err` is not "no institutions".** An unreadable or unparseable value
    /// means the answer is unknown, and Gate D's caller must treat that the way
    /// `KbAffiliation::Unknown` is treated — restrictively — rather than as an
    /// empty set. Returning a `Result` rather than defaulting is what forces
    /// that decision to be made at the gate instead of here.
    pub async fn session_affiliations(
        &self,
        session_id: &str,
    ) -> Result<std::collections::BTreeSet<crate::privacy::affiliation::InstitutionId>> {
        self.storage.session_affiliations(session_id).await
    }

    pub async fn get_session(&self, id: &str, include_messages: bool) -> Result<Session> {
        self.storage.get_session(id, include_messages).await
    }

    /// Resume (or create + bind) a durable session keyed by a stable external
    /// handle such as `"app:<app-id>:<client-id>"`. Returns `(session, resumed)`
    /// where `resumed` is true when an existing session was reused. Backs the
    /// BRSDK durable-app-session feature.
    pub async fn get_or_create_by_external_key(
        &self,
        external_key: &str,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
    ) -> Result<(Session, bool)> {
        self.storage
            .get_or_create_by_external_key(external_key, working_dir, name, session_type)
            .await
    }

    /// Fetch only the session's token counters, without the `COUNT(*)` over the
    /// messages table or deserializing the heavy metadata columns that
    /// `get_session` parses. Used on the per-streamed-event hot path where the
    /// message count and metadata are irrelevant.
    pub async fn get_token_counts(&self, id: &str) -> Result<SessionTokenCounts> {
        self.storage.get_token_counts(id).await
    }

    pub fn update(&self, id: &str) -> SessionUpdateBuilder<'_> {
        SessionUpdateBuilder::new(self, id.to_string())
    }

    /// Read ONE key out of a session's `extension_data`, without the
    /// `COUNT(*)` over `messages` and the metadata deserialization
    /// [`Self::get_session`] pays for. A cleared key (persisted as JSON `null`,
    /// which is how `goal.rs` erases a resolved goal) reads as absent, matching
    /// `RunState::load_from`.
    ///
    /// Returns `Ok(None)` both when the session does not exist and when it has
    /// no value under that key — callers that need to tell those apart should
    /// ask for the session.
    pub async fn get_extension_state(
        &self,
        session_id: &str,
        extension: &str,
        version: &str,
    ) -> Result<Option<serde_json::Value>> {
        self.storage
            .get_extension_state(session_id, extension, version)
            .await
    }

    /// Atomically read-modify-write ONE key of a session's `extension_data`.
    ///
    /// `extension_data` is a single JSON column shared by every per-session
    /// extension (`goal.v0`, `run_state.*`, `todo.v0`, `workspace_skills.v1`).
    /// The older writer shape — `get_session` → mutate the whole
    /// [`ExtensionData`] → `update(id).extension_data(..)` — reads and writes in
    /// two separate statements, so two overlapping writers each serialize a
    /// stale snapshot of the WHOLE object and the later commit silently erases
    /// the earlier one, even when they touched different keys. Tool calls do
    /// overlap (the agent loop drives them through `select_all`), so this is
    /// reachable, not theoretical.
    ///
    /// Here the read and the write happen inside one transaction that OPENS
    /// WITH A WRITE — the same load-bearing trick as
    /// `replace_conversation_inner`: a deferred WAL transaction that reads
    /// first pins a read snapshot and gets an immediate `SQLITE_BUSY_SNAPSHOT`
    /// on upgrade (the busy handler is not consulted for a snapshot upgrade),
    /// whereas taking the single per-file write lock up front makes any SELECT
    /// that follows read true latest-committed state and makes a concurrent
    /// writer wait on the busy timeout instead. Writers are therefore fully
    /// serialized and no merge basis can be stale — no CAS/retry needed.
    ///
    /// `mutate` sees the currently persisted value (`None` when absent or
    /// cleared) and returns the value to store. Returns the stored value, or
    /// `Ok(None)` if no such session exists (nothing is written, and no row is
    /// created). A `mutate` error rolls the transaction back.
    pub async fn update_extension_state<F>(
        &self,
        session_id: &str,
        extension: &str,
        version: &str,
        mutate: F,
    ) -> Result<Option<serde_json::Value>>
    where
        F: FnOnce(Option<&serde_json::Value>) -> Result<serde_json::Value> + Send,
    {
        self.storage
            .update_extension_state(session_id, extension, version, mutate)
            .await
    }

    /// Atomically mutate the complete extension-data object for operations whose
    /// security invariant spans more than one versioned key.
    ///
    /// Most callers should use [`Self::update_extension_state`]. This wider form
    /// exists for migrations that must install a replacement record and remove
    /// its legacy credential-bearing record in the same SQLite transaction. A
    /// mutation error rolls the transaction back; `Ok(None)` means the session
    /// did not exist.
    pub async fn update_extension_data<F, R>(
        &self,
        session_id: &str,
        mutate: F,
    ) -> Result<Option<R>>
    where
        F: FnOnce(&mut ExtensionData) -> Result<R> + Send,
        R: Send,
    {
        self.storage.update_extension_data(session_id, mutate).await
    }

    /// Set the session's working directory **only if the chat is still empty**
    /// (#44), as one atomic conditional `UPDATE`: the emptiness check is the
    /// statement's own `WHERE NOT EXISTS (…messages…)` clause, so a first
    /// message landing concurrently can never slip between a check and a
    /// write — the update either sees no messages and applies, or sees the
    /// message and refuses. (The previous read-count-then-write sequence ran
    /// as two statements and had exactly that TOCTOU window.)
    ///
    /// This closes the *persisted-state* race only. It cannot order itself
    /// against a turn that has been accepted but has not yet persisted its
    /// user message; callers that can (the HTTP route) additionally hold the
    /// per-session turn guard across the update + agent restart.
    pub async fn try_update_working_dir_if_empty(
        &self,
        id: &str,
        working_dir: PathBuf,
    ) -> Result<WorkingDirUpdate> {
        self.storage
            .try_update_working_dir_if_empty(id, &working_dir)
            .await
    }

    /// Set the session's working directory **unconditionally**, bypassing the
    /// empty-chat-only guard of [`Self::try_update_working_dir_if_empty`].
    ///
    /// ONLY for the terminal shell-following path (`biorouter term run`),
    /// where the session's dir intentionally tracks the user's shell cwd
    /// mid-conversation. Every other caller must use the guarded method — a
    /// mid-chat switch breaks the session's own history (#44), which is why
    /// this is the sole unguarded writer and is named to make misuse obvious.
    pub async fn force_update_working_dir_unguarded(
        &self,
        id: &str,
        working_dir: PathBuf,
    ) -> Result<()> {
        let mut builder = self.update(id);
        builder.working_dir = Some(working_dir);
        builder.apply().await
    }

    async fn apply_update_inner(&self, builder: SessionUpdateBuilder<'_>) -> Result<()> {
        self.storage.apply_update(builder).await
    }

    /// Persist one message, returning the **effective** `msg_uid` it was stored
    /// under. Usually the message's own id (or a freshly minted one when the
    /// caller supplied none), but a uid collision re-mints — callers that keep
    /// the message in memory must adopt the returned uid so the in-memory and
    /// persisted ids agree (#41).
    /// Close the underlying SQLite pool, releasing the store's file handles
    /// (the db plus its WAL/-shm siblings). Ordering seam for #31: a private
    /// `--no-session` store's temp directory can only be deleted reliably on
    /// platforms where unlinking open files fails (Windows) if the pool is
    /// closed FIRST. Every later store operation fails with a pool-closed
    /// error, so call this only when the run is done with the store.
    pub async fn close(&self) {
        self.storage.close().await;
    }

    pub async fn add_message(&self, id: &str, message: &Message) -> Result<String> {
        self.storage.add_message(id, message).await
    }

    /// [`Self::add_message`] that also stamps the EFFECTIVE uid onto the
    /// caller's in-memory message (#41). The store mints a uid for idless
    /// messages and re-mints on a collision; a retained copy that keeps
    /// `id: None` (or a stale id) desynchronizes memory from storage — its
    /// next persist would insert a duplicate row under a fresh uid instead
    /// of being recognized as a replay. Use this on every path that both
    /// persists a message AND keeps/yields it.
    pub async fn add_message_adopting_uid(&self, id: &str, message: &mut Message) -> Result<()> {
        let effective_uid = self.storage.add_message(id, message).await?;
        if message.id.as_deref() != Some(effective_uid.as_str()) {
            message.id = Some(effective_uid);
        }
        Ok(())
    }

    /// Unconditional whole-history rewrite: DELETE every message of the session
    /// and re-INSERT the supplied ones.
    ///
    /// This is the NAMED EXCEPTION. It is only correct for a caller that
    /// genuinely owns the whole history — `/clear`, and the import/copy/diverge
    /// paths that write into a session they just created. A caller that
    /// computed `conversation` from a snapshot of a *live* session must use
    /// [`Self::replace_conversation_preserving_tail`] instead: anything another
    /// writer appended in between is destroyed here, silently, after that
    /// writer was already told its append succeeded.
    pub async fn replace_conversation(&self, id: &str, conversation: &Conversation) -> Result<()> {
        self.storage.replace_conversation(id, conversation).await
    }

    /// The current revision of a session's stored message set (see
    /// [`ConversationRevision`]). Cheap: one covering-index aggregate.
    pub async fn conversation_revision(&self, id: &str) -> Result<ConversationRevision> {
        self.storage.conversation_revision(id).await
    }

    /// Snapshot a session for a whole-history rewrite: its conversation plus the
    /// revision that view is based on.
    ///
    /// Reads the REVISION FIRST, then the conversation. A message landing
    /// between the two reads is then inside the returned conversation (so the
    /// rewrite already accounts for it) rather than looking foreign. Reading in
    /// the other order would leave such a message absent from the caller's view
    /// *and* at `id <= max_rowid`, i.e. invisible to tail recovery — a silent
    /// loss. The ordering is load-bearing.
    pub async fn snapshot_for_rewrite(&self, id: &str) -> Result<(Session, ConversationRevision)> {
        let revision = self.storage.conversation_revision(id).await?;
        let session = self.storage.get_session(id, true).await?;
        Ok((session, revision))
    }

    /// Whole-history rewrite that is safe against a concurrent append.
    ///
    /// `known` is the conversation `replacement` was derived from; `basis` is
    /// the revision at the moment that view began (both come from
    /// [`Self::snapshot_for_rewrite`]). Messages stored since `basis` whose ids
    /// are not in `known` are FOREIGN — another writer appended them while this
    /// caller was computing its rewrite — and are carried over onto the end of
    /// `replacement` instead of being destroyed.
    ///
    /// The check runs inside the rewrite's own transaction, under the write
    /// lock its first statement takes, so there is no window between the check
    /// and the DELETE at any timescale.
    ///
    /// Returns what was ACTUALLY stored, so the caller can keep its in-memory
    /// conversation, the database and any `HistoryReplaced` event in agreement.
    /// On [`ReplaceOutcome::Stale`] / [`ReplaceOutcome::SessionNotFound`]
    /// nothing was written and the returned conversation is `replacement`
    /// unchanged.
    ///
    /// Only a genuine basis mismatch is reported as `Stale`. A `SQLITE_BUSY`, an
    /// I/O error or a full disk propagates as `Err` — reporting a busy database
    /// as "stale" would look like data loss, and reporting it as "written"
    /// would be data loss.
    ///
    /// This makes concurrent writes SAFE, not RARE. Two turns on one session
    /// (two app sockets, or the CLI and the daemon on the same `sessions.db`)
    /// still both run; one of them now finds out it lost instead of silently
    /// deleting the other's messages.
    pub async fn replace_conversation_preserving_tail(
        &self,
        id: &str,
        replacement: &Conversation,
        basis: ConversationRevision,
        known: &Conversation,
    ) -> Result<(ReplaceOutcome, Conversation)> {
        self.storage
            .replace_conversation_preserving_tail(id, replacement, basis, known)
            .await
    }

    /// Fetch one externalized tool-result payload by its blob handle (BR-7).
    /// `None` when the handle is unknown to this session — blobs are scoped to
    /// the session that stored them, so one session can never read another's
    /// tool output through a guessed (or model-hallucinated) handle.
    pub async fn get_message_blob(
        &self,
        session_id: &str,
        blob_uid: &str,
    ) -> Result<Option<String>> {
        self.storage.get_message_blob(session_id, blob_uid).await
    }

    pub async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.storage.list_sessions().await
    }

    /// Ids of the sessions whose stored `extension_data` blob mentions
    /// `extension_name` anywhere, so an uninstall can prune the rosters that
    /// still name a package it is about to delete.
    ///
    /// ⚠ **A coarse prefilter, not the decision.** The blob is one JSON column
    /// shared by every per-session extension, so a `LIKE` over it can match a
    /// todo item that happens to quote the name. The caller re-parses each
    /// candidate's `enabled_extensions.v0` and prunes only a real entry; what
    /// this query buys is not having to deserialize eleven thousand sessions to
    /// find the three that matter. False positives are therefore harmless and
    /// false negatives impossible — the roster stores the extension's config,
    /// which carries its name.
    ///
    /// `sub_agent` rows are excluded: their roster is the immutable runtime
    /// profile the run was granted, the same rule
    /// [`crate::agents::session_extensions::record`] enforces on the write side.
    pub async fn sessions_mentioning_extension(&self, extension_name: &str) -> Result<Vec<String>> {
        self.storage
            .sessions_mentioning_extension(extension_name)
            .await
    }

    /// Lightweight session rows for the History sidebar.
    ///
    /// `include_subagents` widens the type filter to `sub_agent` rows (BR-71);
    /// `include_empty` swaps the `messages` INNER JOIN for a LEFT JOIN so a
    /// session that has not yet recorded a message is still listed (used by
    /// `workspace_list`, never by the sidebar).
    pub async fn list_session_summaries(
        &self,
        limit: u32,
        offset: u32,
        include_subagents: bool,
        include_empty: bool,
    ) -> Result<Vec<SessionSummary>> {
        self.storage
            .list_session_summaries(limit, offset, include_subagents, include_empty)
            .await
    }

    pub async fn list_sessions_by_types(&self, types: &[SessionType]) -> Result<Vec<Session>> {
        self.storage.list_sessions_by_types(types).await
    }

    /// [`Self::list_sessions_by_types`], including sessions that have recorded
    /// no message yet.
    ///
    /// The CLI's `session list --subagents` is the only surface in the product
    /// that shows subagent runs, and the default query INNER JOINs `messages` —
    /// so a child that produced nothing was absent from the one listing that
    /// could have found it. See
    /// [`SessionStorage::list_sessions_by_types_maybe_empty`] for why the
    /// sidebar keeps the opposite default.
    pub async fn list_sessions_by_types_including_empty(
        &self,
        types: &[SessionType],
    ) -> Result<Vec<Session>> {
        self.storage
            .list_sessions_by_types_maybe_empty(types, true)
            .await
    }

    pub async fn delete_session(&self, id: &str) -> Result<()> {
        self.storage.delete_session(id).await
    }

    pub async fn clear_all_sessions(&self) -> Result<u64> {
        self.storage.clear_all_sessions().await
    }

    pub async fn count_all_sessions(&self) -> Result<u64> {
        self.storage.count_all_sessions().await
    }

    pub async fn get_insights(&self) -> Result<SessionInsights> {
        self.storage.get_insights().await
    }

    /// Per-day usage for the Home heatmap, over the last `days` calendar days.
    pub async fn get_activity(&self, days: i64) -> Result<ActivityWindow> {
        self.storage.get_activity(days).await
    }

    /// Append one turn's usage to the per-turn token ledger.
    ///
    /// `model` / `provider` attribute the turn for the per-model breakdown; pass
    /// `None` when the provider did not report a model (the row then aggregates
    /// under the 'unknown' group).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_token_event(
        &self,
        session_id: &str,
        input: Option<i32>,
        output: Option<i32>,
        total: i64,
        model: Option<&str>,
        provider: Option<&str>,
        cache_read: Option<i32>,
        cache_creation: Option<i32>,
    ) -> Result<()> {
        self.storage
            .record_token_event(
                session_id,
                input,
                output,
                total,
                model,
                provider,
                cache_read,
                cache_creation,
            )
            .await
    }

    /// Atomically append a production usage event and apply the same event to
    /// the session's lifetime counters. Reusing `event_key` is a no-op, which
    /// makes retrying an ambiguous database result safe.
    pub async fn apply_usage_event(&self, entry: UsageLedgerEntry) -> Result<bool> {
        self.storage.apply_usage_event(entry).await
    }

    /// Per-model usage rollup for one session (for the cost popover breakdown).
    pub async fn get_session_model_usage(&self, session_id: &str) -> Result<Vec<ModelUsageRow>> {
        self.storage.get_session_model_usage(session_id).await
    }

    /// Global per-model usage rollup over `[from, to]` (inclusive, unix seconds).
    pub async fn get_model_usage(&self, from: i64, to: i64) -> Result<Vec<ModelUsageRow>> {
        self.storage.get_model_usage(from, to).await
    }

    /// Queryable, server-priced usage report over `[from, to]` (inclusive, unix
    /// seconds), bucketed by day, model, or day×model.
    pub async fn get_usage_report(
        &self,
        from: i64,
        to: i64,
        group: UsageGroup,
    ) -> Result<Vec<UsageReportRow>> {
        self.storage.get_usage_report(from, to, group).await
    }

    /// Month-to-date + all-time priced usage totals, for the summary gauge.
    pub async fn get_usage_summary(&self) -> Result<UsageSummary> {
        self.storage.get_usage_summary().await
    }

    pub async fn export_session(&self, id: &str) -> Result<String> {
        self.storage.export_session(id).await
    }

    /// Issue #56 — **the privacy gate on exporting a chat.**
    ///
    /// Export had *zero* privacy checks: `biorouter session export <id>` read a
    /// private transcript out of the store and wrote it to a plain file (or to
    /// stdout) with no capability check, no proof of a human, no record, and no
    /// word to the user that the file it just wrote carries none of the
    /// protection the chat had. Every other way out of a private chat — running
    /// a turn on a public model, spawning a public child, reaching a public
    /// connector, declassifying — is gated; a `>` redirect was not.
    ///
    /// ⚠ **This does NOT declassify, and it must never grow into that.** The
    /// ruling is explicit: the chat stays private. `to_classification` on the row
    /// this writes is `private`, the same value as `from_classification`, and
    /// nothing here touches `sessions.privacy_tier`. The ratchet is untouched —
    /// the point of the record is that a copy left the store, not that the
    /// original changed.
    ///
    /// Three conditions, in this order, and the order is the same one
    /// [`crate::privacy::declassify`] establishes for the same reason: the
    /// cheapest and most explicable refusal first, the operating-system dialog
    /// last, so a user is never asked for their password and *then* told they
    /// were never eligible.
    ///
    /// 1. **Capability.** `caller` is the tier of the model the exporting
    ///    process is running (`ProviderTier`), and a public-capability caller may
    ///    not take a private transcript out. This is the same
    ///    caller-capability ≥ target-classification rule
    ///    [`crate::privacy::visibility::may_read`] states, asked at the one place
    ///    bytes leave the store as a file.
    /// 2. **The person at the keyboard**, via the platform's own authentication
    ///    ([`authenticate_export`]). Capability says *which model*; this says
    ///    *who*, and neither substitutes for the other — an agent holding
    ///    `developer__shell` inherits the terminal's capability but cannot
    ///    satisfy a Touch ID / polkit / Windows Hello prompt.
    /// 3. **The record**, written inside the same transaction that re-reads the
    ///    row, so the tier the ledger reports is the tier that was true when the
    ///    export was authorised rather than one read a moment earlier.
    ///
    /// ⚠ **A public chat is [`ExportDecision::Unrestricted`] and costs nothing** —
    /// no prompt, no row. A gate that fired on every export is one people route
    /// around, and DR-16's posture is a condition, not a wall in front of the
    /// user. The master switch (DR-15) turns the whole thing off for the same
    /// reason it turns off every other gate.
    ///
    /// The copy the caller must show is [`EXPORT_NOT_PROTECTED`]; it is a
    /// constant here rather than in each surface so the terminal and the desktop
    /// cannot describe the same file differently.
    ///
    /// # Which doors call this, and which do not
    ///
    /// ⚠ **State this honestly rather than let a reader assume it is universal.**
    /// Three things in this tree turn a stored session into a file, and only one
    /// of them consults this today:
    ///
    /// * `biorouter session export <id>` (`biorouter-cli`'s
    ///   `handle_session_export`) — **calls this**, before it reads the
    ///   transcript.
    /// * `GET /sessions/{id}/export` (`biorouter-server`'s `routes/session.rs`,
    ///   which the desktop's export button drives) — **does NOT call this yet**.
    ///   It is gated by `routes/session_reach.rs`, so a caller must already
    ///   reach the session, but it raises no system-authentication prompt,
    ///   writes no ledger row and shows no copy. Wiring it is one call in that
    ///   handler; this function is deliberately a `SessionManager` method, and
    ///   not private to the CLI, so that call is available.
    /// * `generate_diagnostics` (`session/diagnostics.rs`, driven by `biorouter
    ///   session diagnostics`) — **does NOT call this**, and reaches
    ///   [`Self::export_session`] directly.
    ///
    /// [`Self::export_session`] itself is intentionally left ungated: it is the
    /// storage read, and a hard refusal inside it would take away the desktop's
    /// ability to export a private chat *at all* (the route has no way to pass
    /// an authorisation), which is a worse answer than the one above.
    pub async fn authorize_export(
        &self,
        session_id: &str,
        caller: crate::privacy::ProviderTier,
        authorization: Option<&ExportAuthorization>,
    ) -> Result<ExportDecision> {
        // DR-15's master opt-out, read once, before the store is touched.
        if !crate::privacy::privacy_tiers_enabled() {
            return Ok(ExportDecision::Unrestricted);
        }

        let pool = self.storage.pool().await?;
        let mut tx = pool.begin().await?;

        let Some((raw_tier, reason_before, provider_name)) =
            sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT privacy_tier, privacy_reason, provider_name FROM sessions WHERE id = ?1",
            )
            .bind(session_id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(ExportDecision::SessionNotFound);
        };

        // The tier the READER sees, not the raw bytes: `from_stored` fails
        // closed, so a hand-edited or restored row that no gate in the tree
        // treats as public is not treated as public here either.
        if SessionClassification::from_stored(&raw_tier) == SessionClassification::Public {
            return Ok(ExportDecision::Unrestricted);
        }

        if !caller.is_private() {
            return Ok(ExportDecision::CapabilityRequired);
        }

        // ⚠ `covers`, not merely `is_some`: one authentication may cover a batch,
        // but only the chats its dialog named (DR-20 point 4). `is_some_and`
        // fails closed for a caller that presented none.
        if !authorization.is_some_and(|granted| granted.covers(session_id)) {
            return Ok(ExportDecision::SystemAuthenticationRequired);
        }

        let message_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = ?1")
                .bind(session_id)
                .fetch_one(&mut *tx)
                .await?;

        // `actor`/`actor_kind` are both "user" for the reason
        // `privacy::declassify` records: this daemon has no principal, so the
        // honest thing the row can say is the KIND of actor, and a fabricated
        // username would be worse than none.
        //
        // ⚠ `from` and `to` are BOTH `private`. Every consumer of this table
        // that means "was declassified" keys on `to_classification = 'public'`
        // (see `SessionStorage::NOT_DECLASSIFIED_BY_USER`), so an export row
        // cannot be mistaken for a declassification by anything that reads it.
        sqlx::query(
            "INSERT INTO classification_audit ( \
                session_id, from_classification, to_classification, reason, actor, actor_kind, \
                app_version, provider_name_at_change, privacy_reason_before, \
                message_count_at_change \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(session_id)
        .bind(SessionClassification::Private.as_sql())
        .bind(SessionClassification::Private.as_sql())
        .bind(EXPORTED_BY_USER)
        .bind("user")
        .bind("user")
        .bind(env!("CARGO_PKG_VERSION"))
        .bind(provider_name)
        .bind(reason_before)
        .bind(message_count)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        tracing::info!(
            session_id,
            "a private chat was exported to a file (issue #56); the chat itself is unchanged"
        );
        Ok(ExportDecision::Authorized)
    }
}

/// The `reason` an export writes into `classification_audit`.
///
/// Deliberately not one of `privacy::declassify`'s reasons: nothing about the
/// session changed, and a value that collided with a declassification reason
/// would make the ledger's own history unreadable.
pub const EXPORTED_BY_USER: &str = "exported_by_user";

/// What the user is told before a private chat is written to a file.
///
/// ⚠ **It is about the FILE, not about the chat.** The failure this copy exists
/// to prevent is the user believing the exported markdown inherits the chat's
/// protection: it does not, it is an ordinary file that any model, any tool and
/// any sync client can read. The chat itself is unchanged and the sentence says
/// so, because a user who thinks exporting declassified their chat will
/// declassify it again "properly" and be surprised twice.
///
/// One constant so the terminal and the desktop cannot describe the same file
/// differently.
pub const EXPORT_NOT_PROTECTED: &str =
    "This chat is private. The file this writes is NOT protected: it is an ordinary file on \
     disk, readable by any model, any tool and anything that syncs your folders. The privacy \
     tier does not travel with it. The chat itself stays private, and the export is recorded in \
     the classification ledger.";

/// What [`SessionManager::authorize_export`] decided.
///
/// Named outcomes rather than a `bool`, because a caller has to say something
/// different for each: "you are on a public model", "the operating system did
/// not authenticate you" and "there is no such chat" send a user to three
/// different places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportDecision {
    /// A public chat, or the master switch is off. Export as before: no prompt,
    /// no record, no copy.
    Unrestricted,
    /// A private chat and the exporting process is running a public model.
    CapabilityRequired,
    /// A private chat, the capability is there, and no system authentication
    /// covering this chat was presented.
    SystemAuthenticationRequired,
    /// Everything is satisfied and the ledger row has been written.
    Authorized,
    /// No such session.
    SessionNotFound,
}

/// Proof that the **operating system** authenticated the user for an export, for
/// the chats it named.
///
/// ⚠ **Unforgeable by the language, not by an audit.** The field is private and
/// there is no public constructor: outside this module the only way to obtain
/// one is [`authenticate_export`], which raises a real platform prompt and
/// returns `Ok` for [`crate::privacy::system_auth::AuthOutcome::Approved`] and
/// nothing else. This mirrors `privacy::declassify::SystemAuthorization`
/// exactly, and is a separate type on purpose: an authorisation the user granted
/// for "export this chat" must not be spendable on "make this chat public".
///
/// Not `Clone` and not `Copy`, for DR-20 point 2's reason — a value that could
/// be duplicated could be stashed in a static and spent again next week.
#[derive(Debug)]
pub struct ExportAuthorization {
    session_ids: Vec<String>,
}

impl ExportAuthorization {
    /// Was `session_id` named by the prompt this authorisation came from?
    pub fn covers(&self, session_id: &str) -> bool {
        self.session_ids.iter().any(|id| id == session_id)
    }

    /// The same proof, for tests that exercise the gate rather than a prompt.
    ///
    /// `#[cfg(test)]` and private, so it is absent from every shipped binary and
    /// unnameable outside this file even in a test build — the same shape
    /// `privacy::declassify::SystemAuthorization::for_test` has, and for the same
    /// reason.
    #[cfg(test)]
    fn for_test(session_ids: &[String]) -> Self {
        Self {
            session_ids: session_ids.to_vec(),
        }
    }
}

/// Raise the platform's authentication prompt **once** for an export of
/// `session_ids`.
///
/// ⚠ **Call this LAST, after [`SessionManager::authorize_export`] has answered
/// [`ExportDecision::SystemAuthenticationRequired`]** — i.e. only once the row is
/// really private and the caller's capability really is sufficient. Prompting
/// earlier asks a user for their password and then tells them they were never
/// allowed.
///
/// The sentence the dialog shows says *export*, never *declassify*: a prompt
/// that misdescribes what it authorises is the one thing an authorisation dialog
/// cannot be.
pub async fn authenticate_export(
    session_ids: &[String],
) -> Result<ExportAuthorization, crate::privacy::system_auth::SystemAuthRefusal> {
    // `SystemAuthenticator` is deliberately NOT imported: `prompter()` returns a
    // `&'static dyn SystemAuthenticator`, and a method called on a trait object
    // resolves through the object's own vtable without the trait in scope.
    use crate::privacy::system_auth::{self, AuthRequest};

    let mut named: Vec<String> = session_ids.to_vec();
    named.sort();
    named.dedup();
    let reason = if named.len() == 1 {
        "Write a copy of this private Biorouter chat to a file.".to_string()
    } else {
        format!(
            "Write copies of {} private Biorouter chats to files.",
            named.len()
        )
    };
    let Ok(request) = AuthRequest::new(reason, &named) else {
        // An empty set names nothing, so the prompt could not state what it
        // authorises and the proof would be spendable on nothing. Refused rather
        // than accepted-and-ignored.
        return Err(system_auth::SystemAuthRefusal {
            outcome: system_auth::AuthOutcome::Denied,
            message: "No chats were named, so there was nothing to authenticate for and \
                      nothing was exported."
                .to_string(),
        });
    };

    let prompter = system_auth::prompter();
    match system_auth::authenticate_or_refuse(prompter, &request).await {
        None => Ok(ExportAuthorization {
            session_ids: request.session_ids,
        }),
        Some(refusal) => Err(refusal),
    }
}

impl SessionManager {
    pub async fn import_session(&self, json: &str) -> Result<Session> {
        self.storage.import_session(self, json).await
    }

    pub async fn copy_session(&self, session_id: &str, new_name: String) -> Result<Session> {
        self.storage.copy_session(self, session_id, new_name).await
    }

    /// Diverge before an edited user message, using the standard divergence
    /// naming and lineage rules while preserving the edit flow's truncation
    /// semantics.
    pub async fn diverge_session_for_edit(
        &self,
        session_id: &str,
        timestamp: i64,
    ) -> Result<Session> {
        self.storage
            .diverge_session_for_edit(self, session_id, timestamp)
            .await
    }

    /// Diverge (branch) a session: copy the full conversation into a fresh
    /// session that records its lineage (`diverged_from`) and gets a
    /// human-friendly, collision-free branch name.
    ///
    /// Naming:
    /// - `custom_name` (when non-blank) is used verbatim.
    /// - Otherwise the name is `"{base} (branch {N})"`, where `base` is the
    ///   parent's name (a placeholder like "New chat" is replaced with a
    ///   title derived from the conversation) with any existing `(branch K)`
    ///   suffix stripped, and `N` is the next free index across that family.
    ///
    /// The branch name is locked (`user_set_name = true`) so the auto-namer
    /// never overwrites the marker. Shared by the `/sessions/{id}/diverge`
    /// route and the CLI/TUI `/diverge` command.
    ///
    /// The branch conversation is trimmed to end at the last *complete*
    /// assistant answer (see `branch_conversation_at`), so a diverge
    /// triggered while the agent is still generating or calling tools never
    /// leaves a dangling, unanswered turn in the new session. `anchor_ms` (the
    /// `created` timestamp of the message a per-message Diverge button was
    /// clicked on) bounds the branch to that point; `None` uses the most recent
    /// complete answer.
    pub async fn diverge_session(
        &self,
        session_id: &str,
        custom_name: Option<String>,
        anchor_ms: Option<i64>,
    ) -> Result<Session> {
        self.diverge_session_at(session_id, custom_name, anchor_ms, None)
            .await
    }

    /// Diverge anchored by a durable message id (`anchor_uid`), the BR-45 divergence
    /// point. Preferred over the timestamp anchor: it is unambiguous when two
    /// messages share a whole second, and it records `branch_point_msg_uid` on
    /// the child. `anchor_ms` is kept as a back-compatible fallback for clients
    /// that still pass a timestamp.
    pub async fn diverge_session_at(
        &self,
        session_id: &str,
        custom_name: Option<String>,
        anchor_ms: Option<i64>,
        anchor_uid: Option<String>,
    ) -> Result<Session> {
        self.storage
            .diverge_session(self, session_id, custom_name, anchor_ms, anchor_uid)
            .await
    }

    /// Drop every message at or after `timestamp` — checkpoint restore and the
    /// message-edit flow.
    ///
    /// The range is open above, so it also takes anything appended between the
    /// caller reading the conversation and this call landing. That is only
    /// sound for a caller that owns the whole tail (the edit flow's
    /// just-created divergence). A caller working from a snapshot of a LIVE
    /// session should hold on to the revision it read and use
    /// [`Self::truncate_conversation_bounded`], which cuts only as far as that
    /// view reached.
    pub async fn truncate_conversation(&self, session_id: &str, timestamp: i64) -> Result<()> {
        self.storage
            .truncate_conversation(session_id, timestamp)
            .await
    }

    /// [`Self::truncate_conversation`] bounded by the caller's own view.
    ///
    /// `basis` is the revision the decision to cut at `timestamp` was made
    /// from (see [`Self::snapshot_for_rewrite`] /
    /// [`Self::conversation_revision`]). Messages stored above its watermark
    /// were appended after that view and are KEPT: they are not part of the
    /// tail the caller asked to drop, and their writer has already been told
    /// the append succeeded.
    ///
    /// A basis from a previous incarnation of this session id is refused
    /// outright — its watermark describes rowids that belonged to a different
    /// conversation (see [`ConversationRevision`]).
    pub async fn truncate_conversation_bounded(
        &self,
        session_id: &str,
        timestamp: i64,
        basis: ConversationRevision,
    ) -> Result<TruncateOutcome> {
        self.storage
            .truncate_conversation_bounded(session_id, timestamp, basis)
            .await
    }

    // BR-43 shadow-git checkpoints: `checkpoints` table access, delegated to
    // `SessionStorage` (which owns the pool) and called by `CheckpointManager`.

    pub async fn insert_checkpoint(&self, rec: &crate::checkpoint::CheckpointRecord) -> Result<()> {
        self.storage.insert_checkpoint(rec).await
    }

    pub async fn list_checkpoints(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::checkpoint::CheckpointRecord>> {
        self.storage.list_checkpoints(session_id).await
    }

    pub async fn last_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::checkpoint::CheckpointRecord>> {
        self.storage.last_checkpoint(session_id).await
    }

    pub async fn get_checkpoint(
        &self,
        session_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<crate::checkpoint::CheckpointRecord>> {
        self.storage.get_checkpoint(session_id, checkpoint_id).await
    }

    pub async fn delete_checkpoints(&self, session_id: &str) -> Result<()> {
        self.storage.delete_checkpoints(session_id).await
    }

    pub async fn maybe_update_name(&self, id: &str, provider: Arc<dyn Provider>) -> Result<()> {
        let session = self.get_session(id, true).await?;

        // The user explicitly named the session — never override.
        if session.user_set_name {
            return Ok(());
        }

        // Whether the session is still on a placeholder title. A session that
        // already has a real, content-derived name should stop regenerating it
        // after the first few turns (no churn); a session still showing the
        // "New chat" placeholder must keep getting a chance to be named on
        // every turn, no matter how long it grows — otherwise an early naming
        // miss (e.g. an interrupted or errored first turn) leaves it stuck on
        // "New chat" forever once it crosses the message-count threshold.
        let still_default = is_default_session_name(&session.name);

        let conversation = session
            .conversation
            .ok_or_else(|| anyhow::anyhow!("No messages found"))?;

        let user_message_count = conversation
            .messages()
            .iter()
            .filter(|m| matches!(m.role, Role::User))
            .count();

        // No real exchange yet — nothing to name from.
        if user_message_count == 0 {
            return Ok(());
        }

        // After the first few exchanges the name has settled; stop regenerating
        // it so later turns don't churn the title — but only once it actually
        // has a real name. While still on the placeholder, keep trying.
        if user_message_count > MSG_COUNT_FOR_SESSION_NAME_GENERATION && !still_default {
            return Ok(());
        }

        // Prefer the LLM-generated, content-derived name. The naming call is
        // best-effort: if the provider errors (rate limit, auth, model issue)
        // or hands back an empty/whitespace string, fall back to a
        // deterministic title derived from the first user message so a session
        // is NEVER left as "New chat" after a real exchange.
        let name = match provider.generate_session_name(&conversation).await {
            Ok(name) if !name.trim().is_empty() => name,
            Ok(_) => {
                warn!(
                    "Session name generation for {} returned an empty name; using fallback",
                    id
                );
                Self::fallback_session_name(&conversation)
            }
            Err(e) => {
                warn!(
                    "Session name generation for {} failed ({}); using fallback",
                    id, e
                );
                Self::fallback_session_name(&conversation)
            }
        };

        // Both the LLM and the fallback produced nothing usable (e.g. the first
        // user message was only attachments). Leave the placeholder rather than
        // blanking the name.
        if name.trim().is_empty() {
            return Ok(());
        }

        self.update(id).system_generated_name(name).apply().await
    }

    /// Derive a short, deterministic session title from the first user message.
    /// Used as a fallback when the LLM-based namer is unavailable so a session
    /// with a real exchange never stays as the "New chat" placeholder.
    fn fallback_session_name(conversation: &Conversation) -> String {
        let first_user_text = conversation
            .messages()
            .iter()
            .find(|m| matches!(m.role, Role::User))
            .map(|m| m.as_concat_text())
            .unwrap_or_default();

        // Collapse whitespace and keep the leading words so the title fits the
        // limited UI space (mirrors the LLM namer's "4 words or less" intent).
        let snippet = first_user_text
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");

        crate::utils::safe_truncate(&snippet, 60)
    }

    /// Issue #56 Gate D. `caller_capability` is the reach the *reader* has, not
    /// the classification of the session doing the searching: with
    /// [`ProviderTier::Public`] the storage layer filters private sessions out
    /// in SQL, before the `LIMIT` is applied.
    pub async fn search_chat_history(
        &self,
        query: &str,
        limit: Option<usize>,
        after_date: Option<chrono::DateTime<chrono::Utc>>,
        before_date: Option<chrono::DateTime<chrono::Utc>>,
        exclude_session_id: Option<String>,
        // Issue #56 Gate D and DR-26 / Task 50 Step 3: both axes together — see
        // `chat_history_search::SearchReach`.
        reach: crate::session::chat_history_search::SearchReach,
    ) -> Result<crate::session::chat_history_search::ChatRecallResults> {
        self.storage
            .search_chat_history(
                query,
                limit,
                after_date,
                before_date,
                exclude_session_id,
                reach,
            )
            .await
    }
}

pub struct SessionStorage {
    pool: Pool<Sqlite>,
    initialized: tokio::sync::OnceCell<()>,
    session_dir: PathBuf,
}

/// How `replace_conversation_inner` treats concurrent writers.
enum RewriteGuard<'a> {
    /// Overwrite whatever is there. Only for a caller that owns the whole
    /// history: `/clear`, and writes into a session it just created.
    Unconditional,
    /// Refuse if the basis moved out from under us, and carry over anything
    /// appended since it that the caller never saw.
    PreserveTail {
        basis: ConversationRevision,
        known: &'a Conversation,
    },
}

fn role_to_string(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: String::new(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            name: String::new(),
            user_set_name: false,
            session_type: SessionType::default(),
            created_at: Default::default(),
            updated_at: Default::default(),
            extension_data: ExtensionData::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: None,
            workflow: None,
            user_workflow_values: None,
            conversation: None,
            message_count: 0,
            provider_name: None,
            model_config: None,
            diverged_from: None,
            branch_point_msg_uid: None,
            parent_session_id: None,
            privacy_tier: SessionClassification::Public,
            privacy_reason: None,
        }
    }
}

impl Session {
    pub fn without_messages(mut self) -> Self {
        self.conversation = None;
        self
    }
}

/// True when `name` is a placeholder title (empty, "New chat", legacy "New
/// Session", "CLI Session", "New session N", or "Session N") rather than a meaningful name —
/// mirrors the frontend `isDefaultSessionName`. Used so a diverged branch
/// doesn't inherit a useless placeholder.
pub(crate) fn is_default_session_name(name: &str) -> bool {
    let n = name.trim();
    if n.is_empty() {
        return true;
    }
    if n.eq_ignore_ascii_case(DEFAULT_SESSION_NAME)
        || n.eq_ignore_ascii_case("New Session")
        || n.eq_ignore_ascii_case("CLI Session")
    {
        return true;
    }
    // "New session <N>" or "Session <N>" (trailing digits).
    let lower = n.to_ascii_lowercase();
    for prefix in ["new session ", "session "] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn canonical_session_name(name: String, user_set_name: bool) -> String {
    if !user_set_name && is_default_session_name(&name) {
        DEFAULT_SESSION_NAME.to_string()
    } else {
        name
    }
}

/// Strip a trailing `" (branch <digits>)"` from a name so branching a branch
/// re-numbers within the same family instead of nesting suffixes
/// ("Foo (branch 1)" → "Foo", then the next branch becomes "Foo (branch 2)").
pub(crate) fn strip_branch_suffix(name: &str) -> &str {
    let trimmed = name.trim_end();
    if let Some(idx) = trimmed.rfind(" (branch ") {
        let (base, suffix) = trimmed.split_at(idx);
        let inner = suffix.strip_prefix(" (branch ").unwrap_or(suffix);
        if let Some(digits) = inner.strip_suffix(')') {
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                return base.trim_end();
            }
        }
    }
    trimmed
}

/// Escape SQL LIKE wildcards in a literal so a name containing `%` or `_`
/// (or `\`) is matched literally. Pair with `ESCAPE '\'`.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// True when `m` is a *complete* assistant answer: an assistant message that
/// carries real text and has no pending tool call. These are the only points a
/// branch should end on — everything after the last one (an unanswered user
/// question, an empty "about to call a tool" assistant turn, a tool
/// request/response still mid-flight) is an in-progress exchange.
fn is_assistant_terminal_answer(m: &Message) -> bool {
    m.role == Role::Assistant && !m.is_tool_call() && !m.as_concat_text().trim().is_empty()
}

/// Where a branch's history ends, resolved from whatever anchor the client
/// supplied.
///
/// A branch is always a **prefix** of its parent, so every variant names a cut
/// point rather than a predicate. [`BranchCut::Whole`] is also the deliberate
/// landing place for an anchor that cannot be trusted — see
/// [`resolve_branch_anchor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchCut {
    /// Keep every message.
    Whole,
    /// Keep `msgs[..=idx]`.
    Through(usize),
    /// Keep nothing: the anchor predates the conversation.
    Nothing,
}

/// Resolve the branch point a client asked for against the conversation as it
/// is actually stored.
///
/// The governing rule is issue #167's: **a branch that keeps too much is
/// recoverable; one that discards the conversation is not.** Every ambiguous or
/// unverifiable case therefore widens the branch rather than narrowing it.
///
/// `anchor_uid` is the durable `Message.id` of the assistant answer the
/// per-message Branch control was clicked on; `anchor_ms` is that same
/// message's `created`. The desktop sends both, and this uses the second to
/// CHECK the first:
///
/// * **Unresolvable id → keep everything.** The client's view of a live turn is
///   structurally short of the store (`chatStreamStore.tsx`'s
///   `viewNamesEveryStoredRow`: one streamed reply becomes two or three rows and
///   only the first keeps the id the client was shown; the `MessagesPersisted`
///   frame that publishes the rest is not consumed). An id we cannot find is
///   therefore an ordinary, expected state — and it is not evidence about
///   *where* the user clicked, so it must not silently become a timestamp cut.
///   That silent degradation is what BR-45 left behind and what this replaces.
/// * **An id that resolves EARLIER than the timestamp says → keep everything.**
///   This is #167 itself. The reporter pressed Branch late in a long chat and
///   got a branch holding only the first exchange, because the id their client
///   held for that message was one storage had given to the first reply. The
///   two anchors describe ONE message, so `created < anchor_ms` proves they
///   disagree and the id is the stale half. Note the comparison is strict: the
///   rows one reply is split into share (or postdate) its `created`, so an
///   honest anchor never trips this.
/// * **A duplicated id → the LAST match, never the first.** Storage enforces
///   `UNIQUE(session_id, msg_uid)`, so this cannot arise from a stored
///   conversation; it can from one assembled in memory, and `position()`'s
///   first-match would cut at the earliest of them.
///
/// With no `anchor_uid` at all the legacy timestamp anchor applies, as a
/// **prefix** — the last index created at or before `anchor_ms`. It is
/// deliberately not a filter over the whole conversation: `created` is not
/// monotonic in real chats (a compaction writes its summary rows with the
/// current time and leaves the `/compact` request behind them carrying an older
/// one), and a filter keeps late-but-older messages while dropping
/// early-but-newer ones — producing a branch with holes in it, and orphaning
/// tool responses from their requests.
fn resolve_branch_anchor(
    msgs: &[Message],
    anchor_uid: Option<&str>,
    anchor_ms: Option<i64>,
) -> BranchCut {
    let Some(uid) = anchor_uid else {
        // Legacy clients (and the CLI) send only a timestamp. Cut at the last
        // message at or before it; a conversation whose every message postdates
        // the anchor cuts to nothing.
        return match anchor_ms {
            Some(ts) => msgs
                .iter()
                .rposition(|m| m.created <= ts)
                .map_or(BranchCut::Nothing, BranchCut::Through),
            None => BranchCut::Whole,
        };
    };

    let Some(idx) = msgs.iter().rposition(|m| m.id.as_deref() == Some(uid)) else {
        warn!(
            anchor_uid = %uid,
            messages = msgs.len(),
            "branch anchor names a message this conversation does not hold; \
             branching from the end of the conversation instead of guessing"
        );
        return BranchCut::Whole;
    };

    if let Some(ts) = anchor_ms {
        if msgs[idx].created < ts {
            warn!(
                anchor_uid = %uid,
                anchored_at = msgs[idx].created,
                clicked_at = ts,
                "branch anchor id resolves to a message older than the anchor \
                 timestamp; treating the id as stale and branching from the end \
                 of the conversation (issue #167)"
            );
            return BranchCut::Whole;
        }
    }

    BranchCut::Through(idx)
}

/// Build a diverged branch's conversation: the parent's history, cut at the
/// anchor the client asked for, then trimmed back to the last complete
/// assistant answer inside that cut. A diverge fired while the agent is still
/// generating or calling tools therefore branches from the previous finished
/// response rather than leaving a dangling, unanswered turn.
///
/// The cut comes from [`resolve_branch_anchor`], which prefers the durable
/// message id and falls back to keeping the whole conversation — never to a
/// narrower guess — when that id cannot be trusted. With no anchor at all the
/// most recent complete answer in the whole conversation is used. If there is
/// no complete answer within the cut (e.g. diverged before the very first reply
/// landed), the branch starts empty rather than carrying an orphaned question.
///
/// Returns the branch conversation together with the id of the message the
/// anchor was actually honoured at, or `None` when no anchor was honoured. The
/// caller records that as `branch_point_msg_uid`; the two are computed here, in
/// one place, so a refused anchor can never be written to the row as though it
/// had been used (issue #167).
pub(crate) fn branch_conversation_at(
    conversation: &Conversation,
    anchor_uid: Option<&str>,
    anchor_ms: Option<i64>,
) -> (Conversation, Option<String>) {
    let msgs = conversation.messages();
    let cut = resolve_branch_anchor(msgs, anchor_uid, anchor_ms);
    let honoured = match cut {
        BranchCut::Through(idx) => msgs[idx].id.clone(),
        BranchCut::Whole | BranchCut::Nothing => None,
    };
    (trim_to_last_complete_answer_cut(msgs, cut), honoured)
}

/// The second half of the trim: take the prefix `cut` names, then back up to the
/// last complete assistant answer inside it.
fn trim_to_last_complete_answer_cut(msgs: &[Message], cut: BranchCut) -> Conversation {
    let kept: &[Message] = match cut {
        BranchCut::Nothing => &[],
        BranchCut::Through(end) => msgs.get(..=end).unwrap_or(msgs),
        BranchCut::Whole => msgs,
    };

    match kept.iter().rposition(is_assistant_terminal_answer) {
        Some(end) => Conversation::new_unvalidated(kept[..=end].to_vec()),
        None => Conversation::default(),
    }
}

/// The fail-closed read of `sessions.privacy_tier`, shared by every row reader
/// so the two cannot drift.
///
/// This deliberately does NOT follow the `try_get(..).ok().flatten()` convention
/// the optional columns beside it use: a projection that omits the column is a
/// bug, and issue #56 would rather paint every row Private (loudly, and visibly
/// to the user on day one) than quietly hand back Public.
fn read_privacy_tier(row: &sqlx::sqlite::SqliteRow) -> SessionClassification {
    use sqlx::Row;
    row.try_get::<String, _>("privacy_tier")
        .map(|s| SessionClassification::from_stored(&s))
        .unwrap_or_else(|_| {
            tracing::error!("privacy_tier missing from projection; reading Private");
            SessionClassification::Private
        })
}

/// What the one-time backfill did, in the four buckets a support conversation
/// needs. Returned as well as logged, so a test can assert the numbers without
/// reading a process-global log stream — see `fn_body` in this file's tests for
/// why that distinction had to be made.
///
/// The first three partition the `sessions` table. `empty` cuts across it: it
/// counts the message-less rows, which is the whole of the gap between these
/// figures and the ones the user can see, because History hides them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BackfillCounts {
    /// Rows this run raised to Private, from a bound private-tier provider.
    pub private: i64,
    /// Rows this run raised to Private from the **turn ledger** — `token_events`
    /// recorded a billed turn served by a private-tier provider, and the row's
    /// `provider_name` (the LAST binding) did not say so.
    ///
    /// This is the population finding 9 is about, counted rather than described:
    /// a chat that ran on Ollama and was then switched to Claude for one
    /// formatting question. `provider_name` reads `anthropic`; the ledger still
    /// holds the Ollama turns. Disjoint from [`Self::private`] by construction —
    /// the last-bound statement runs first, so anything it raised is no longer
    /// `privacy_tier = 'public'` when the ledger statement runs.
    pub private_from_turn_history: i64,
    /// Rows that matched a raise on the evidence but were **skipped because the
    /// user declassified them**. Not an error; the number exists so a support
    /// conversation can tell "the migration found nothing" apart from "the
    /// migration found it and correctly left it alone".
    pub declassified_skipped: i64,
    /// Rows left public with a provider name recorded — a read, not a guess.
    pub public_named: i64,
    /// Rows left public with NO provider recorded. Unknown provenance, failed
    /// open (DR-10). Historically the largest of the three.
    pub unknown_provider: i64,
    /// Rows with no messages at all, whatever their tier.
    pub empty: i64,
}

/// The numbers issue #56's day-one notice quotes (§15.5), over the population
/// **History actually shows**.
///
/// That qualifier is the whole point of the type. The raw `sessions` table holds
/// message-less rows — a chat opened and abandoned, a workspace shell — and
/// `list_sessions_by_types` INNER JOINs `messages`, so those rows exist in the
/// database and nowhere in the user's window. Measured on the operator's
/// 8,415-row database on 2026-08-03, the two populations differ by more than 2×:
/// **1,486** rows the backfill would raise, **654** of them visible. A notice
/// quoting the raw number tells the user to go and act on 832 conversations they
/// cannot see.
///
/// ⚠ Those are a measurement with a date on it, not a constant. §16's table has
/// been re-measured three times during this work and moved by a factor of three
/// in four days; a figure in this comment is stale the moment the user opens
/// another chat. Nothing reads them — every number the user sees is computed —
/// and any figure quoted here must carry the date it was taken.
///
/// The first three fields partition [`Self::total_visible`]: private (whatever
/// the migration or a later turn marked), then the public remainder split by
/// whether the row records a provider at all. The unknown bucket is the one the
/// backfill fails **open** on, and it is the largest of the three.
/// `pub(crate)`, matching [`Self::privacy_notice_counts`], its only producer. A
/// `pub` struct an external crate can name but never obtain is a wart; when the
/// HTTP route that serves these numbers lands, both widen together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PrivacyNoticeCounts {
    /// Private to the fail-closed reader, and visible in History.
    pub private_visible: i64,
    /// Public, visible, and bound to a named provider — the migration read that
    /// name and concluded public.
    pub public_named_visible: i64,
    /// Public, visible, and bound to NO provider. The migration could not tell,
    /// and DR-10 says fail open. These are the conversations of genuinely
    /// unknown provenance.
    pub unknown_provider_visible: i64,
    /// The denominator: user + scheduled sessions with at least one message.
    pub total_visible: i64,
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for SessionSummary {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let user_set_name = row.try_get("user_set_name").unwrap_or(false);
        let name = canonical_session_name(row.try_get("name")?, user_set_name);

        Ok(SessionSummary {
            id: row.try_get("id")?,
            working_dir: row.try_get("working_dir")?,
            name,
            user_set_name,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            message_count: row.try_get("message_count")?,
            parent_session_id: row.try_get("parent_session_id").ok().flatten(),
            session_type: row.try_get("session_type").ok().flatten(),
            // Tolerant, like the two above: a SELECT that omits the column
            // yields None rather than erroring.
            diverged_from: row.try_get("diverged_from").ok().flatten(),
            privacy_tier: read_privacy_tier(row),
        })
    }
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for Session {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let workflow_json: Option<String> = row.try_get("workflow_json")?;
        let workflow = workflow_json.and_then(|json| serde_json::from_str(&json).ok());

        let user_workflow_values_json: Option<String> = row.try_get("user_workflow_values_json")?;
        let user_workflow_values =
            user_workflow_values_json.and_then(|json| serde_json::from_str(&json).ok());

        let model_config_json: Option<String> = row.try_get("model_config_json").ok().flatten();
        let model_config = model_config_json.and_then(|json| serde_json::from_str(&json).ok());

        let stored_name: String = {
            let name_val: String = row.try_get("name").unwrap_or_default();
            if !name_val.is_empty() {
                name_val
            } else {
                row.try_get("description").unwrap_or_default()
            }
        };

        let user_set_name = row.try_get("user_set_name").unwrap_or(false);
        let name = canonical_session_name(stored_name, user_set_name);

        let session_type_str: String = row
            .try_get("session_type")
            .unwrap_or_else(|_| "user".to_string());
        let session_type = session_type_str.parse().unwrap_or_default();

        Ok(Session {
            id: row.try_get("id")?,
            working_dir: PathBuf::from(row.try_get::<String, _>("working_dir")?),
            name,
            user_set_name,
            session_type,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            extension_data: serde_json::from_str(&row.try_get::<String, _>("extension_data")?)
                .unwrap_or_default(),
            total_tokens: row.try_get("total_tokens")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            accumulated_total_tokens: row.try_get("accumulated_total_tokens")?,
            accumulated_input_tokens: row.try_get("accumulated_input_tokens")?,
            accumulated_output_tokens: row.try_get("accumulated_output_tokens")?,
            schedule_id: row.try_get("schedule_id")?,
            workflow,
            user_workflow_values,
            conversation: None,
            message_count: row.try_get("message_count").unwrap_or(0) as usize,
            provider_name: row.try_get("provider_name").ok().flatten(),
            model_config,
            diverged_from: row.try_get("diverged_from").ok().flatten(),
            // Tolerant read: SELECTs that omit the column (e.g. the session
            // list) yield None rather than erroring, mirroring `model_config`.
            // The privacy tier below deliberately does NOT follow this
            // convention — see `SessionClassification::from_stored`.
            branch_point_msg_uid: row.try_get("branch_point_msg_uid").ok().flatten(),
            parent_session_id: row.try_get("parent_session_id").ok().flatten(),
            privacy_tier: read_privacy_tier(row),
            privacy_reason: row.try_get("privacy_reason").ok().flatten(),
        })
    }
}

/// What [`SessionStorage::bind_provider_if_allowed`] did.
///
/// Three outcomes, because two of them are indistinguishable by `rows_affected`
/// and they mean different things to a caller: a bind that the privacy ratchet
/// refused is a 409 the user can act on, and a bind against an id that names no
/// row is a bug or a stale client. A `bool` return collapses them, and the
/// collapse runs in the dangerous direction — a mistyped id would be reported to
/// the user as "this chat is private".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindOutcome {
    Bound,
    RefusedByPrivacy,
    NoSuchSession,
}

impl SessionStorage {
    /// See [`SessionManager::record_session_affiliation`]. One statement, so a
    /// concurrent recorder cannot lose an institution between a read and a
    /// write.
    async fn record_session_affiliation(&self, session_id: &str, institution: &str) -> Result<()> {
        let pool = self.pool().await?;
        // DR-15 / AR-7: with the master opt-out off nothing is recorded, for the
        // reason the tier ratchet stops — this column is monotone and
        // re-enabling never revisits a row, so a ratchet that kept firing would
        // be a deferred, permanent impact on a feature the user turned off.
        if !crate::privacy::privacy_tiers_enabled() {
            return Ok(());
        }
        // The `ORDER BY` is cosmetic and nothing may come to depend on it:
        // SQLite does not formally guarantee that an aggregate consumes an
        // ordered subquery in that order. It is here so the stored JSON is
        // stable for a human reading the row; every reader
        // (`SessionStorage::session_affiliations`) collects into a `BTreeSet`,
        // so the set is the value and the sequence is not.
        sqlx::query(
            r#"
            UPDATE sessions
               SET session_affiliations = (
                     SELECT json_group_array(value) FROM (
                       SELECT DISTINCT value
                         FROM json_each(COALESCE(NULLIF(session_affiliations, ''), '[]'))
                       UNION
                       SELECT ?2
                       ORDER BY value
                     )
                   )
             WHERE id = ?1
            "#,
        )
        .bind(session_id)
        .bind(institution)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// See [`SessionManager::session_affiliations`].
    async fn session_affiliations(
        &self,
        session_id: &str,
    ) -> Result<std::collections::BTreeSet<crate::privacy::affiliation::InstitutionId>> {
        let pool = self.pool().await?;
        let raw: Option<String> =
            sqlx::query_scalar("SELECT session_affiliations FROM sessions WHERE id = ?1")
                .bind(session_id)
                .fetch_optional(pool)
                .await?
                .flatten();
        let Some(raw) = raw.filter(|s| !s.is_empty()) else {
            // NULL, absent or empty: this chat has touched no institution's
            // extension, or predates the column. The Missing direction — a fact,
            // not an unknown.
            return Ok(Default::default());
        };
        let ids: Vec<String> = serde_json::from_str(&raw)?;
        Ok(ids
            .iter()
            .map(|id| crate::privacy::affiliation::InstitutionId::new(id))
            .collect())
    }

    /// Bind a provider, atomically refusing a public one on a private session
    /// (issue #56, Gate A).
    ///
    /// The predicate is in the `WHERE`, not in Rust, so a concurrent ratchet
    /// cannot interleave into "private session, public provider bound". The
    /// natural implementation — `SELECT privacy_tier`, decide, `UPDATE` — has a
    /// window between the read and the write that a ratchet fits inside, and
    /// nothing about its shape says so.
    ///
    /// ⚠ **The test rendezvous belongs HERE, immediately before `.execute`, and
    /// nowhere else.** Its contract is *"after every read this function
    /// performs, before the statement that writes"*, and that is the only
    /// position from which it can distinguish this implementation from the wrong
    /// one. Called from `Agent::update_provider` instead — before this function
    /// is even entered — a read-then-write helper passes: the forced ratchet
    /// commits first and the helper's own read dutifully refuses. If a future
    /// refactor gives this function a read of its own, the seam call moves down
    /// below it; it never moves up.
    ///
    /// ⚠ `rows_affected == 0` is NOT the refusal on its own: an id that names no
    /// row produces exactly the same zero. Returning `RefusedByPrivacy` there
    /// would surface a stale or mistyped id as a 409 privacy refusal, and would
    /// make Gate A's first test pass for the wrong reason against a stale
    /// fixture id — the one way that test can lie. The two are distinguished
    /// with a single follow-up read in the zero case, on a path that is already
    /// an error.
    pub(crate) async fn bind_provider_if_allowed(
        &self,
        session_id: &str,
        provider_name: &str,
        model_config_json: &str,
        incoming_is_private: bool,
    ) -> Result<BindOutcome> {
        let pool = self.pool().await?;
        // DR-15's master opt-out, read INSIDE the gate rather than through an
        // `is_enabled()` wrapper, so a mid-session change is honoured and the
        // opt-out is one auditable line rather than an absent gate.
        //
        // It is folded into the bound parameter rather than branching around the
        // statement, so the toggle cannot change WHICH statement runs — the
        // atomicity argument in this function's doc comment is about that one
        // `UPDATE … WHERE`, and a second code path would need its own.
        //
        // A direct read, not a `CallCapability`: a provider bind is not a tool
        // call and has no admitted capability to inherit.
        let admits_anything = incoming_is_private || !crate::privacy::privacy_tiers_enabled();
        let write = sqlx::query(
            r#"
            UPDATE sessions
               SET provider_name = ?, model_config_json = ?, updated_at = datetime('now')
             WHERE id = ?
               AND (privacy_tier = 'public' OR ? = 1)
            "#,
        )
        .bind(provider_name)
        .bind(model_config_json)
        .bind(session_id)
        .bind(i64::from(admits_anything));
        // ⚠ The last statement before the write, and test-only. Read the ⚠ in
        //    this function's doc comment before moving it.
        #[cfg(test)]
        crate::agents::agent::seams::before_bind_write().await;
        let res = write.execute(pool).await?;
        if res.rows_affected() > 0 {
            return Ok(BindOutcome::Bound);
        }
        // Zero rows: either the row is private and the provider is public, or
        // there is no row with that id at all.
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
            .bind(session_id)
            .fetch_one(pool)
            .await?;
        Ok(if exists {
            BindOutcome::RefusedByPrivacy
        } else {
            BindOutcome::NoSuchSession
        })
    }

    /// The four fields the session-row change feed compares, for a set of ids.
    ///
    /// One statement rather than one per id, and deliberately NOT `get_session`:
    /// that reads the whole row and, without `metadata_only`, the transcript
    /// with it — this runs on a timer for every chat any client has open.
    ///
    /// ⚠ `updated_at` is neither selected nor compared, and that is the
    /// load-bearing omission. `insert_message` and the token-usage update both
    /// stamp it, so a diff that included it would fire many times per turn —
    /// the same defect as watching the database file's mtime, wearing a
    /// column's clothes. See [`crate::session_meta`].
    ///
    /// An id with no row is simply absent from the result: the feed reports
    /// changes to rows, and a deleted chat is not a changed one.
    pub async fn session_meta_rows(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<crate::session_meta::SessionMetaRow>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let pool = self.pool().await?;
        let placeholders = std::iter::repeat_n("?", session_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, provider_name, \
                    json_extract(model_config_json, '$.model_name') AS model_name, \
                    privacy_tier, privacy_reason \
               FROM sessions WHERE id IN ({placeholders})"
        );
        let mut query = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(&sql);
        for id in session_ids {
            query = query.bind(id);
        }
        Ok(query
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(
                |(session_id, provider_name, model_name, privacy_tier, privacy_reason)| {
                    crate::session_meta::SessionMetaRow {
                        session_id,
                        provider_name,
                        model_name,
                        privacy_tier,
                        privacy_reason,
                    }
                },
            )
            .collect())
    }

    /// Persist one lead/worker routing snapshot only while the session still
    /// names the exact composite configuration that produced it.
    ///
    /// This is one conditional write rather than a read followed by an update:
    /// a user can select another provider while the old turn is settling, and
    /// that newer bind must win even when it uses the same provider name.
    pub(crate) async fn update_composite_model_config_if_generation_matches(
        &self,
        session_id: &str,
        expected_provider_name: &str,
        expected_generation: &str,
        model_config_json: &str,
    ) -> Result<bool> {
        let pool = self.pool().await?;
        let write = sqlx::query(
            r#"
            UPDATE sessions
               SET model_config_json = ?, updated_at = datetime('now')
             WHERE id = ?
               AND provider_name = ?
               AND json_extract(
                     model_config_json,
                     '$.request_params.__biorouter_provider_restore.config_generation'
                   ) = ?
            "#,
        )
        .bind(model_config_json)
        .bind(session_id)
        .bind(expected_provider_name)
        .bind(expected_generation);
        #[cfg(test)]
        crate::agents::agent::seams::before_composite_state_write().await;
        Ok(write.execute(pool).await?.rows_affected() == 1)
    }

    fn create_pool(path: &Path) -> Pool<Sqlite> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create session database directory");
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .busy_timeout(std::time::Duration::from_secs(5))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            // Under WAL, NORMAL is durable across application crashes and only
            // risks the last commit on an OS/power crash, while avoiding an
            // fsync on every commit (the SQLite default is FULL). This removes
            // a per-message-write fsync from the agent hot path.
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

        // SQLite serializes writes on a single write lock; fanning out to many
        // writer connections just produces lock contention rather than
        // parallelism. Cap the pool deliberately instead of inheriting sqlx's
        // default of 10.
        SqlitePoolOptions::new()
            .max_connections(4)
            .connect_lazy_with(options)
    }

    pub fn new(data_dir: PathBuf) -> Self {
        let session_dir = data_dir.join(SESSIONS_FOLDER);
        let db_path = session_dir.join(DB_NAME);
        Self {
            pool: Self::create_pool(&db_path),
            initialized: tokio::sync::OnceCell::new(),
            session_dir,
        }
    }

    /// Close the SQLite pool (see [`SessionManager::close`]). Safe to call
    /// even if the lazy pool never connected; idempotent.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// The migrated pool. `pub(crate)` rather than private because
    /// [`crate::privacy::declassify`] owns the ONE statement in the tree that
    /// lowers `privacy_tier`, and it deliberately does not go through
    /// [`SessionUpdateBuilder`] — the builder's monotone `CASE WHEN` cannot
    /// express a lowering, and giving it a way to would hand every caller in the
    /// tree the same ability. Still crate-internal: no route, no MCP tool and no
    /// CLI command can reach a raw connection through this.
    pub(crate) async fn pool(&self) -> Result<&Pool<Sqlite>> {
        self.initialized
            .get_or_try_init(|| async {
                let schema_exists = sqlx::query_scalar::<_, bool>(
                    r#"SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='schema_version')"#,
                )
                .fetch_one(&self.pool)
                .await
                .unwrap_or(false);

                if schema_exists {
                    Self::run_migrations(&self.pool).await?;
                } else {
                    Self::create_schema(&self.pool).await?;
                    if let Err(e) = Self::import_legacy(&self.pool, &self.session_dir).await {
                        warn!("Failed to import some legacy sessions: {}", e);
                    }
                    // ⚠ Issue #56 finding 10 — **the classification backfill must
                    // run HERE too, and this branch is the reason it is not
                    // enough to own the numbered arms.**
                    //
                    // `create_schema` stamps `CURRENT_SCHEMA_VERSION` before this
                    // line, so `run_migrations` will never enter arm 19 or arm 20
                    // on this database in any later process either. Every legacy
                    // JSONL chat therefore arrived Public and stayed Public:
                    // `import_legacy_session` binds `session.privacy_tier`, which
                    // deserialises from a file written years before the column
                    // existed and so takes `#[serde(default =
                    // "SessionClassification::public")]`.
                    //
                    // The innocent path is not exotic — it is `sessions.db` being
                    // lost or reset (a support instruction, a Reset App Data, a
                    // machine move). The user's Ollama history comes back public,
                    // silently, on a database that reports itself fully migrated.
                    //
                    // ⚠ Failure here must NOT be logged and forgotten, which is the
                    // shape the rest of this branch uses. `create_schema` has
                    // already stamped the counter at the top, so a swallowed
                    // failure leaves imported rows public on a database that no
                    // arm will ever re-enter — the same silent-public outcome, one
                    // level down. It is also not worth aborting a first launch
                    // over. So the counter is walked back below the repair arm and
                    // the next launch retries there. `retreat_to_privacy_repair`
                    // owns that, and says why it is safe.
                    if let Err(e) = Self::backfill_privacy_from_recorded_provenance(&self.pool).await
                    {
                        warn!(
                            "issue #56: the privacy backfill failed on the fresh-database import \
                             path; imported legacy chats stay public until it succeeds: {e}"
                        );
                        Self::retreat_to_privacy_repair(&self.pool).await;
                    }
                }
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        Ok(&self.pool)
    }

    pub async fn create(session_dir: &Path) -> Result<Self> {
        let storage = Self::new(session_dir.to_path_buf());
        Self::create_schema(&storage.pool).await?;
        Ok(storage)
    }

    async fn create_schema(pool: &Pool<Sqlite>) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        "#,
        )
        .execute(pool)
        .await?;

        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(CURRENT_SCHEMA_VERSION)
            .execute(pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                user_set_name BOOLEAN DEFAULT FALSE,
                session_type TEXT NOT NULL DEFAULT 'user',
                working_dir TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                extension_data TEXT DEFAULT '{}',
                total_tokens INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                accumulated_total_tokens INTEGER,
                accumulated_input_tokens INTEGER,
                accumulated_output_tokens INTEGER,
                schedule_id TEXT,
                workflow_json TEXT,
                user_workflow_values_json TEXT,
                provider_name TEXT,
                model_config_json TEXT,
                diverged_from TEXT,
                parent_session_id TEXT,
                privacy_tier TEXT NOT NULL DEFAULT 'public',
                privacy_reason TEXT,
                session_affiliations TEXT,
                external_key TEXT,
                branch_point_msg_uid TEXT,
                incarnation INTEGER NOT NULL DEFAULT 0
            )
        "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_timestamp INTEGER NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                tokens INTEGER,
                metadata_json TEXT,
                msg_uid TEXT
            )
        "#,
        )
        .execute(pool)
        .await?;

        Self::create_usage_schema(pool).await?;

        // BR-43 shadow-git checkpoints (migration 13), created inline for fresh DBs.
        Self::create_checkpoints_table(pool).await?;

        // BR-7 externalized tool-result payloads (migration 16).
        Self::create_message_blobs_table(pool).await?;

        // #56 declassification ledger (migration 18), created inline for fresh
        // DBs. The privacy columns are already in the `sessions` DDL above; this
        // is the other half of the same migration's shape.
        Self::create_classification_audit_table(pool).await?;

        // #56 Task 49 (DR-26): the user's cross-affiliation grants. Created here
        // for a fresh database and by `ensure_privacy_schema` for every existing
        // one; see the DDL's own comment for why it consumes no migration number.
        sqlx::query(CROSS_AFFILIATION_GRANTS_DDL)
            .execute(pool)
            .await?;

        sqlx::query("CREATE INDEX idx_messages_session ON messages(session_id)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_messages_timestamp ON messages(timestamp)")
            .execute(pool)
            .await?;
        // BR-45: durable per-message id, unique within a session (ids are
        // intentionally carried into a diverged child, so uniqueness is
        // per-session, not global).
        sqlx::query("CREATE UNIQUE INDEX idx_messages_uid ON messages(session_id, msg_uid)")
            .execute(pool)
            .await?;

        // FTS5 index for relevance-ranked chat recall (BR-17). See migration 15
        // for details; a fresh DB starts already indexed.
        sqlx::query(MESSAGES_FTS_DDL).execute(pool).await?;
        sqlx::query("CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_sessions_type ON sessions(session_type)")
            .execute(pool)
            .await?;
        // BRSDK: stable external handle for durable, resumable app sessions
        // (e.g. "app:<app-id>:<client-id>"). Unique so a reconnecting client
        // resolves back to its existing session.
        sqlx::query(
            "CREATE UNIQUE INDEX idx_sessions_external_key ON sessions(external_key) WHERE external_key IS NOT NULL",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    async fn create_usage_schema(pool: &Pool<Sqlite>) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE token_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                ts INTEGER NOT NULL,
                input_tokens INTEGER,
                output_tokens INTEGER,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                billed_total_tokens INTEGER,
                model_id TEXT,
                provider TEXT,
                cache_read_tokens INTEGER,
                cache_creation_tokens INTEGER,
                event_key TEXT,
                session_type TEXT
            )
        "#,
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX idx_token_events_ts ON token_events(ts)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_token_events_session ON token_events(session_id, ts)")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX idx_token_events_event_key ON token_events(event_key) WHERE event_key IS NOT NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(DELETED_CHAT_USAGE_DDL).execute(pool).await?;
        Ok(())
    }

    async fn import_legacy(pool: &Pool<Sqlite>, session_dir: &PathBuf) -> Result<()> {
        use crate::session::legacy;

        let sessions = match legacy::list_sessions(session_dir) {
            Ok(sessions) => sessions,
            Err(_) => {
                warn!("No legacy sessions found to import");
                return Ok(());
            }
        };

        if sessions.is_empty() {
            return Ok(());
        }

        let mut imported_count = 0;
        let mut failed_count = 0;

        for (session_name, session_path) in sessions {
            match legacy::load_session(&session_name, &session_path) {
                Ok(session) => match Self::import_legacy_session(pool, &session).await {
                    Ok(_) => {
                        imported_count += 1;
                        info!("  ✓ Imported: {}", session_name);
                    }
                    Err(e) => {
                        failed_count += 1;
                        info!("  ✗ Failed to import {}: {}", session_name, e);
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    info!("  ✗ Failed to load {}: {}", session_name, e);
                }
            }
        }

        info!(
            "Import complete: {} successful, {} failed",
            imported_count, failed_count
        );
        Ok(())
    }

    async fn import_legacy_session(pool: &Pool<Sqlite>, session: &Session) -> Result<()> {
        let mut tx = pool.begin().await?;

        let workflow_json = match &session.workflow {
            Some(workflow) => Some(serde_json::to_string(workflow)?),
            None => None,
        };

        let user_workflow_values_json = match &session.user_workflow_values {
            Some(user_workflow_values) => Some(serde_json::to_string(user_workflow_values)?),
            None => None,
        };

        let model_config_json = match &session.model_config {
            Some(model_config) => Some(serde_json::to_string(model_config)?),
            None => None,
        };

        sqlx::query(
            r#"
        INSERT INTO sessions (
            id, name, user_set_name, session_type, working_dir, created_at, updated_at, extension_data,
            total_tokens, input_tokens, output_tokens,
            accumulated_total_tokens, accumulated_input_tokens, accumulated_output_tokens,
            schedule_id, workflow_json, user_workflow_values_json,
            provider_name, model_config_json, diverged_from, parent_session_id,
            privacy_tier, privacy_reason, incarnation
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, random())
        "#,
        )
        .bind(&session.id)
        .bind(&session.name)
        .bind(session.user_set_name)
        .bind(session.session_type.to_string())
        .bind(session.working_dir.to_string_lossy().as_ref())
        .bind(session.created_at)
        .bind(session.updated_at)
        .bind(serde_json::to_string(&session.extension_data)?)
        .bind(session.total_tokens)
        .bind(session.input_tokens)
        .bind(session.output_tokens)
        .bind(session.accumulated_total_tokens)
        .bind(session.accumulated_input_tokens)
        .bind(session.accumulated_output_tokens)
        .bind(&session.schedule_id)
        .bind(workflow_json)
        .bind(user_workflow_values_json)
        .bind(&session.provider_name)
        .bind(model_config_json)
        .bind(&session.diverged_from)
        .bind(&session.parent_session_id)
        .bind(session.privacy_tier.as_sql())
        .bind(&session.privacy_reason)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        if let Some(conversation) = &session.conversation {
            Self::replace_conversation_inner(
                pool,
                &session.id,
                conversation,
                RewriteGuard::Unconditional,
            )
            .await?;

            // ...and put the historical mtime back. `replace_conversation_inner`
            // opens with `UPDATE sessions SET updated_at = datetime('now')` —
            // that write IS how the transaction takes SQLite's write lock up
            // front, so it is not optional and it beats the back-dated value
            // this function just INSERTed. Right for a live rewrite, wrong for
            // an import: without this every legacy JSONL session would sort as
            // "just now" under `ORDER BY updated_at DESC`, collapsing a user's
            // whole history to today in the session list on the one migration
            // run that imports it.
            sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
                .bind(session.updated_at)
                .bind(&session.id)
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    async fn run_migrations(pool: &Pool<Sqlite>) -> Result<()> {
        let current_version = Self::get_schema_version(pool).await?;

        if current_version < CURRENT_SCHEMA_VERSION {
            info!(
                "Running database migrations from v{} to v{}...",
                current_version, CURRENT_SCHEMA_VERSION
            );

            for version in (current_version + 1)..=CURRENT_SCHEMA_VERSION {
                info!("  Applying migration v{}...", version);
                Self::apply_migration(pool, version).await?;
                Self::update_schema_version(pool, version).await?;
                info!("  ✓ Migration v{} complete", version);
            }

            info!("All migrations complete");
        }

        // Development builds shipped overlapping v11-v14 migration numbers for
        // the usage and loop feature branches. Reconcile both additive schemas
        // from their actual table shapes so those databases upgrade safely even
        // when a version number caused one branch's migration arm to be skipped.
        Self::reconcile_usage_schema(pool).await?;
        Self::reconcile_loop_schema(pool).await?;

        Ok(())
    }

    async fn reconcile_loop_schema(pool: &Pool<Sqlite>) -> Result<()> {
        Self::create_checkpoints_table(pool).await?;
        Self::ensure_message_identity_schema(pool).await?;
        Self::ensure_session_incarnation_schema(pool).await?;
        Self::ensure_privacy_schema(pool).await?;
        Self::create_and_backfill_messages_fts(pool, false).await?;
        Self::create_message_blobs_table(pool).await?;
        // Last: every table it sweeps exists by now.
        Self::retire_side_rows_of_deleted_chats(pool).await?;
        Ok(())
    }

    /// Retire the rows earlier builds left behind when they deleted a chat —
    /// the rows [`Self::delete_chat_side_rows`] now removes with it.
    ///
    /// ⚠ **A startup sweep, not a numbered migration arm**, for the reasons
    /// [`Self::prune_orphaned_token_events`] gives: it is idempotent, safe in
    /// either merge order, and keeps repairing what a build without the delete
    /// fix leaves while it shares this file. `BEGIN IMMEDIATE`, so no message,
    /// checkpoint or grant can be written between a test here and its delete.
    async fn retire_side_rows_of_deleted_chats(pool: &Pool<Sqlite>) -> Result<()> {
        let mut connection = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        let result = Self::retire_side_rows_of_deleted_chats_locked(&mut connection).await;
        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *connection).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                Err(error)
            }
        }
    }

    async fn retire_side_rows_of_deleted_chats_locked(
        connection: &mut sqlx::SqliteConnection,
    ) -> Result<()> {
        // Recall text is tested against its MESSAGE, not its chat. A row mirrors
        // exactly one message, and message rowids are AUTOINCREMENT — never
        // minted twice — so "its message still exists" is exact, and it also
        // finds a deleted chat's text filed under an id that is live again,
        // which existence of the chat cannot. Every writer inserts the row in
        // its message's own transaction, and this holds the write lock, so no
        // live row can be caught between the two. It is a full scan of the
        // index: 0.12–0.13 s end to end, measured in a fresh process against a
        // 557 MB file holding 100,000 rows and 200 MB of indexed text.
        let recall = if Self::messages_fts_exists(&mut *connection).await {
            sqlx::query(
                "DELETE FROM messages_fts WHERE message_id NOT IN (SELECT id FROM messages)",
            )
            .execute(&mut *connection)
            .await?
            .rows_affected()
        } else {
            0
        };

        // Checkpoints are tested on existence alone, as F10's usage rows are:
        // one left under an id that is live again cannot be told from the new
        // chat's own without a timestamp guess, and a wrong guess would delete
        // somebody's undo point for good.
        let checkpoints = sqlx::query(
            "DELETE FROM checkpoints \
              WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id = checkpoints.session_id)",
        )
        .execute(&mut *connection)
        .await?
        .rows_affected();

        // Grants get the timestamp test that checkpoints do not, for the
        // opposite reason: a grant recorded before the chat now holding its id
        // cannot be that chat's, and deleting one by mistake costs the user one
        // question asked again — the reader already refuses exactly these
        // rows (`GRANT_IS_THE_CHATS_OWN`), so this changes nothing a gate sees.
        //
        // Shape-guarded like the rest of the reconcile: a `sessions` table
        // without `created_at` (the experimental v11–v14 shapes) must cost the
        // age test, not the startup.
        let sessions_have_created_at: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'created_at'",
        )
        .fetch_one(&mut *connection)
        .await?;
        let live_grant = if sessions_have_created_at > 0 {
            crate::privacy::grant::GRANT_IS_THE_CHATS_OWN
        } else {
            "1 = 1"
        };
        let grants = sqlx::query(&format!(
            "DELETE FROM cross_affiliation_grants \
              WHERE rowid NOT IN (SELECT g.rowid FROM cross_affiliation_grants g \
                                    JOIN sessions s ON s.id = g.session_id \
                                   WHERE {live_grant})"
        ))
        .execute(&mut *connection)
        .await?
        .rows_affected();

        if recall + checkpoints + grants > 0 {
            info!(
                recall,
                checkpoints, grants, "Retired side rows whose chat no longer exists"
            );
        }
        Ok(())
    }

    async fn reconcile_usage_schema(pool: &Pool<Sqlite>) -> Result<()> {
        // BEGIN IMMEDIATE serializes the check-then-ALTER sequence across
        // concurrently running Biorouter processes. A deferred transaction lets
        // two readers both observe a missing column before either ALTERs it.
        let mut connection = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        let result = Self::reconcile_usage_schema_locked(&mut connection).await;
        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *connection).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                Err(error)
            }
        }
    }

    async fn reconcile_usage_schema_locked(connection: &mut sqlx::SqliteConnection) -> Result<()> {
        for (column, sql_type) in [
            ("model_id", "TEXT"),
            ("provider", "TEXT"),
            ("cache_read_tokens", "INTEGER"),
            ("cache_creation_tokens", "INTEGER"),
            ("billed_total_tokens", "INTEGER"),
            ("event_key", "TEXT"),
            ("session_type", "TEXT"),
        ] {
            let exists: i32 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('token_events') WHERE name = ?1",
            )
            .bind(column)
            .fetch_one(&mut *connection)
            .await?;
            if exists == 0 {
                sqlx::query(&format!(
                    "ALTER TABLE token_events ADD COLUMN {column} {sql_type}"
                ))
                .execute(&mut *connection)
                .await?;
            }
        }

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_token_events_event_key ON token_events(event_key) WHERE event_key IS NOT NULL",
        )
        .execute(&mut *connection)
        .await?;

        sqlx::query(DELETED_CHAT_USAGE_DDL)
            .execute(&mut *connection)
            .await?;
        Self::prune_orphaned_token_events(connection).await?;

        // Capture the session classification while it still exists. The prune
        // above has already removed every row whose chat is gone, so this only
        // classifies rows of live chats; a row left unclassified would still be
        // excluded from user/subagent spend rather than assumed billable.
        sqlx::query(
            r#"
            UPDATE token_events
            SET session_type = (
                SELECT s.session_type FROM sessions s WHERE s.id = token_events.session_id
            )
            WHERE session_type IS NULL
            "#,
        )
        .execute(&mut *connection)
        .await?;

        // Old v11/v12 development migrations copied each session's final model
        // backward and sometimes materialized unknown cache buckets as zero.
        // Without either a durable event identity or a billed total, there is no
        // trustworthy evidence that those values describe the original call.
        sqlx::query(
            r#"
            UPDATE token_events
            SET model_id = NULL,
                provider = NULL,
                cache_read_tokens = NULL,
                cache_creation_tokens = NULL
            WHERE event_key IS NULL
              AND billed_total_tokens IS NULL
              AND (model_id IS NOT NULL
                   OR provider IS NOT NULL
                   OR cache_read_tokens IS NOT NULL
                   OR cache_creation_tokens IS NOT NULL)
            "#,
        )
        .execute(&mut *connection)
        .await?;

        Ok(())
    }

    /// Retire every `token_events` row whose chat no longer exists (F10): fold
    /// its billable usage into `deleted_chat_usage`, exactly as `delete_session`
    /// does, then delete it.
    ///
    /// The production writer, `apply_usage_event`, only inserts beside the
    /// `sessions` row it updates in the same transaction, and `delete_session`
    /// now retires a chat's ledger together with the chat. What this repairs is
    /// every row an earlier delete left behind — and any an older build still
    /// leaves while it shares this file, which the desktop daemon, a terminal
    /// `biorouter` and a scheduled job all open. Folding before deleting is what
    /// makes the upgrade invisible in the Usage panel: every token it counted
    /// before, it still counts; only the per-chat trail is gone.
    ///
    /// ⚠ **A startup sweep, not a numbered migration arm, deliberately.** An arm
    /// consumes a migration number, and this file already records both ways
    /// that goes wrong: development branches have collided on numbers (v11–v14,
    /// and 17), and work added to a number a tester's database has already
    /// passed never runs there (the O10 hazard, arms 18–20). A second open finds
    /// nothing to fold or delete, so running this on every open is the same
    /// one-time repair on a database that has orphans, a standing one for the
    /// mixed-build case above, and safe in either merge order. It runs inside
    /// `reconcile_usage_schema`'s `BEGIN IMMEDIATE`, so the fold and the delete
    /// land together and can never see a turn half-recorded.
    ///
    /// ⚠ **Existence is the whole test**, so a row whose `session_id` names a
    /// live chat is never touched, however old. That leaves one residual this
    /// cannot see: an orphan whose id `create_session` has already re-issued
    /// now reads as the new chat's. Telling the two apart would take a
    /// timestamp guess (a row older than its chat's `created_at`) run
    /// destructively on every open — the wrong trade for a population that
    /// measured zero on a real 11,780-chat store, and one the delete path
    /// closes for every chat deleted from now on.
    async fn prune_orphaned_token_events(connection: &mut sqlx::SqliteConnection) -> Result<()> {
        sqlx::query(&Self::fold_into_deleted_chat_usage_sql(
            "NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id = te.session_id)",
        ))
        .execute(&mut *connection)
        .await?;
        let pruned = sqlx::query(
            "DELETE FROM token_events \
              WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id = token_events.session_id)",
        )
        .execute(&mut *connection)
        .await?
        .rows_affected();
        if pruned > 0 {
            info!("Retired {pruned} token_events rows whose chat no longer exists");
        }
        Ok(())
    }

    /// The one statement that folds the billable `token_events` rows matching
    /// `filter` — a predicate over `te` — into `deleted_chat_usage`, one bucket
    /// per local day, model and provider, adding into any bucket already there.
    /// Shared by `delete_session` and the startup sweep, each of which deletes
    /// the rows it folded inside the same transaction.
    ///
    /// Only billable turns are kept, because only the totals over
    /// [`BILLABLE_USAGE`] read this table: a hidden, terminal or unclassified
    /// row was never counted by any of them, so it goes without a trace.
    fn fold_into_deleted_chat_usage_sql(filter: &str) -> String {
        format!(
            "INSERT INTO deleted_chat_usage \
                 (day, model_id, provider, turns, \
                  input_tokens, input_known, output_tokens, output_known, \
                  billed_total_tokens, billed_known, cache_read_tokens, cache_read_known, \
                  cache_creation_tokens, cache_creation_known) \
             SELECT date(te.ts, 'unixepoch', 'localtime'), \
                    COALESCE(te.model_id, ''), COALESCE(te.provider, ''), COUNT(*), \
                    COALESCE(SUM(te.input_tokens), 0), COUNT(te.input_tokens), \
                    COALESCE(SUM(te.output_tokens), 0), COUNT(te.output_tokens), \
                    COALESCE(SUM(te.billed_total_tokens), 0), COUNT(te.billed_total_tokens), \
                    COALESCE(SUM(te.cache_read_tokens), 0), COUNT(te.cache_read_tokens), \
                    COALESCE(SUM(te.cache_creation_tokens), 0), COUNT(te.cache_creation_tokens) \
               FROM token_events te \
              WHERE te.session_type IN ('user', 'scheduled', 'sub_agent') AND ({filter}) \
              GROUP BY 1, 2, 3 \
             ON CONFLICT (day, model_id, provider) DO UPDATE SET \
                 turns = turns + excluded.turns, \
                 input_tokens = input_tokens + excluded.input_tokens, \
                 input_known = input_known + excluded.input_known, \
                 output_tokens = output_tokens + excluded.output_tokens, \
                 output_known = output_known + excluded.output_known, \
                 billed_total_tokens = billed_total_tokens + excluded.billed_total_tokens, \
                 billed_known = billed_known + excluded.billed_known, \
                 cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens, \
                 cache_read_known = cache_read_known + excluded.cache_read_known, \
                 cache_creation_tokens = cache_creation_tokens + excluded.cache_creation_tokens, \
                 cache_creation_known = cache_creation_known + excluded.cache_creation_known"
        )
    }

    async fn get_schema_version(pool: &Pool<Sqlite>) -> Result<i32> {
        let table_exists = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT name FROM sqlite_master
                WHERE type='table' AND name='schema_version'
            )
        "#,
        )
        .fetch_one(pool)
        .await?;

        if !table_exists {
            return Ok(0);
        }

        let version = sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM schema_version")
            .fetch_one(pool)
            .await?;

        Ok(version)
    }

    async fn update_schema_version(pool: &Pool<Sqlite>, version: i32) -> Result<()> {
        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(version)
            .execute(pool)
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_migration(pool: &Pool<Sqlite>, version: i32) -> Result<()> {
        match version {
            1 => {
                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS schema_version (
                        version INTEGER PRIMARY KEY,
                        applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    )
                "#,
                )
                .execute(pool)
                .await?;
            }
            2 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN user_workflow_values_json TEXT
                "#,
                )
                .execute(pool)
                .await?;
            }
            3 => {
                sqlx::query(
                    r#"
                    ALTER TABLE messages ADD COLUMN metadata_json TEXT
                "#,
                )
                .execute(pool)
                .await?;
            }
            4 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN name TEXT DEFAULT ''
                "#,
                )
                .execute(pool)
                .await?;

                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN user_set_name BOOLEAN DEFAULT FALSE
                "#,
                )
                .execute(pool)
                .await?;
            }
            5 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN session_type TEXT NOT NULL DEFAULT 'user'
                "#,
                )
                .execute(pool)
                .await?;

                sqlx::query("CREATE INDEX idx_sessions_type ON sessions(session_type)")
                    .execute(pool)
                    .await?;
            }
            6 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN provider_name TEXT
                "#,
                )
                .execute(pool)
                .await?;

                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN model_config_json TEXT
                "#,
                )
                .execute(pool)
                .await?;
            }
            7 => {
                // Rename pre-v1.50.0 columns: recipe_json → workflow_json
                // and user_recipe_values_json → user_workflow_values_json
                let recipe_col_count: i32 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'recipe_json'",
                )
                .fetch_one(pool)
                .await?;

                if recipe_col_count > 0 {
                    sqlx::query("ALTER TABLE sessions RENAME COLUMN recipe_json TO workflow_json")
                        .execute(pool)
                        .await?;
                }

                let user_recipe_col_count: i32 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'user_recipe_values_json'",
                )
                .fetch_one(pool)
                .await?;

                if user_recipe_col_count > 0 {
                    sqlx::query(
                        "ALTER TABLE sessions RENAME COLUMN user_recipe_values_json TO user_workflow_values_json",
                    )
                    .execute(pool)
                    .await?;
                }
            }
            8 => {
                // Lineage pointer for diverged (branched) sessions.
                sqlx::query("ALTER TABLE sessions ADD COLUMN diverged_from TEXT")
                    .execute(pool)
                    .await?;
            }
            9 => {
                // BRSDK durable sessions: a stable external handle so an app
                // client can resume its session across reconnects.
                sqlx::query("ALTER TABLE sessions ADD COLUMN external_key TEXT")
                    .execute(pool)
                    .await?;
                sqlx::query(
                    "CREATE UNIQUE INDEX idx_sessions_external_key ON sessions(external_key) WHERE external_key IS NOT NULL",
                )
                .execute(pool)
                .await?;
            }
            10 => {
                // Per-turn token accounting.
                //
                // Before this, tokens existed only as one lifetime total per
                // session with a single created_at/updated_at, so there was no
                // way to answer "how many tokens did I use on Tuesday?" — and
                // the "past 7 days" tile summed the whole lifetime of any
                // session merely *touched* in the window.
                //
                // This is an append-only side table, deliberately NOT
                // `messages.tokens`: `replace_conversation` DELETEs and
                // re-inserts the whole message list, which would drop or
                // re-stamp historical token rows on every edit.
                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS token_events (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL,
                        ts INTEGER NOT NULL,
                        input_tokens INTEGER,
                        output_tokens INTEGER,
                        total_tokens INTEGER NOT NULL DEFAULT 0
                    )
                "#,
                )
                .execute(pool)
                .await?;

                sqlx::query("CREATE INDEX idx_token_events_ts ON token_events(ts)")
                    .execute(pool)
                    .await?;
                sqlx::query(
                    "CREATE INDEX idx_token_events_session ON token_events(session_id, ts)",
                )
                .execute(pool)
                .await?;

                // Seed history so the heatmap is not empty before instrumentation
                // landed. Each pre-existing session's lifetime total is attributed
                // wholesale to the day it was created — the only anchor that
                // exists, and a stable one (created_at never moves).
                sqlx::query(
                    r#"
                    INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens)
                    SELECT id,
                           CAST(strftime('%s', created_at) AS INTEGER),
                           accumulated_input_tokens,
                           accumulated_output_tokens,
                           COALESCE(accumulated_total_tokens, total_tokens, 0)
                    FROM sessions
                    WHERE COALESCE(accumulated_total_tokens, total_tokens, 0) > 0
                "#,
                )
                .execute(pool)
                .await?;
            }
            11 => {
                // Per-turn model attribution. Before this, `token_events` recorded
                // only token counts, so a thread that switched models mid-way (the
                // reported UCSF workflow) could not be split per model — the
                // `ProviderUsage.model` was dropped at record time.
                //
                // Guard each ADD COLUMN with a pragma check so re-running the
                // migration on a DB that already has the column is a no-op.
                let model_col: i32 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM pragma_table_info('token_events') WHERE name = 'model_id'",
                )
                .fetch_one(pool)
                .await?;
                if model_col == 0 {
                    sqlx::query("ALTER TABLE token_events ADD COLUMN model_id TEXT")
                        .execute(pool)
                        .await?;
                }

                let provider_col: i32 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM pragma_table_info('token_events') WHERE name = 'provider'",
                )
                .fetch_one(pool)
                .await?;
                if provider_col == 0 {
                    sqlx::query("ALTER TABLE token_events ADD COLUMN provider TEXT")
                        .execute(pool)
                        .await?;
                }

                // Historical rows deliberately remain NULL. A session stores
                // only its final model/provider, so copying that value backward
                // would fabricate attribution for sessions that switched models.
            }
            12 => {
                // This branch handles a normal v11 → v12 upgrade. The same
                // additive reconciliation also runs unconditionally after the
                // version loop for databases created by early v12 builds.
                Self::reconcile_usage_schema(pool).await?;
            }
            13 => {
                // BR-43 shadow-git checkpoints. Additive side table keyed by the
                // turn's anchor `created_timestamp` (NOT the positional message
                // id) so checkpoints survive the stable-UUID migration.
                Self::create_checkpoints_table(pool).await?;
            }
            14 => {
                // BR-45: stable, durable per-message ids plus an exact branch
                // divergence point. Shape guards make this safe when an experimental
                // v12 database already applied the same feature.
                Self::ensure_message_identity_schema(pool).await?;
            }
            15 => {
                // BR-17 relevance-ranked chat recall. Rebuilding the derived
                // index prevents duplicate rows when an experimental v13/v14
                // database already contains the FTS table and backfill.
                Self::create_and_backfill_messages_fts(pool, true).await?;
            }
            16 => {
                // BR-7 externalized tool-result payload storage. Existing
                // inline messages remain untouched.
                Self::create_message_blobs_table(pool).await?;
            }
            17 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN parent_session_id TEXT
                "#,
                )
                .execute(pool)
                .await?;
            }
            18 => {
                // Shape-guarded, unlike every arm above it, because migration
                // numbers have collided across development branches before —
                // the arm directly above this one is BR-71's, and it landed
                // while issue #56 was still calling this number 17. A raw ALTER
                // on a column another build already created is `duplicate
                // column name`, which aborts startup.
                Self::ensure_privacy_schema(pool).await?;
            }
            19 => {
                // ⚠ **A SEPARATE arm from 18, and this is not tidiness.** The
                // columns landed in 18 while this task was still being written,
                // so every database that ran a build from that window — every
                // developer's, and the operator's, whose `schema_version` records
                // 18 applied on 2026-08-01 — already stands AT 18 and would never
                // re-enter that arm. Folding the backfill in beside the columns
                // therefore shipped a statement that could not run on any machine
                // that had opened the branch: 1,486 chats stranded public, on the
                // very machine the feature was measured against. That is issue
                // #56's own O10 hazard (a migration number consumed by testers)
                // recurring one arm later, and the fix is the same one the tree
                // already applies to the columns — do not add new work to a
                // number someone has already passed.
                //
                // `ensure_privacy_schema` is called again here, and it is not
                // redundant: this arm must not assume 18 ran. A database that
                // reached 18 by a route that skipped it (the same collision the
                // helper exists for) would otherwise hit `no such column:
                // privacy_tier` and abort startup. The helper is idempotent and
                // shape-guarded, so the second call is free.
                Self::ensure_privacy_schema(pool).await?;
                // ...and the backfill. See the function's own comment for why a
                // startup-repeating home would still be wrong even now that the
                // statement is declassification-guarded. The counts it returns are
                // logged inside it; nothing on this path branches on them.
                let _ = Self::backfill_privacy_from_recorded_provenance(pool).await?;
            }
            20 => {
                // ⚠ **The repair arm, and it exists because arm 19 already ran on
                // the databases that matter most.**
                //
                // Arm 19 shipped a backfill that read exactly one column —
                // `sessions.provider_name`, the LAST provider the row was bound
                // to. A chat that ran its history on Ollama and was then switched
                // to Claude for one formatting question reads `anthropic` there
                // and was backfilled **public**, with a private transcript. That
                // is issue #56 finding 9, and it is not a "going forward" problem:
                // the rows are already on disk, on every database that has opened
                // this branch — including the operator's, whose `schema_version`
                // records 19 applied on 2026-08-03.
                //
                // Arm 19 cannot be edited to fix them. That is the O10 hazard this
                // file already records twice: work added to a number a machine has
                // passed runs on no machine that has passed it. So the widened
                // backfill needs a number nobody has passed, which is this one.
                //
                // It is the SAME function as arm 19's, deliberately. A database
                // arriving from ≤18 runs it twice in one ladder, and that is the
                // point — the statements are idempotent (`AND privacy_tier =
                // 'public'`) and declassification-guarded, so "ran twice" is a
                // property this arm proves on every upgrade rather than a hazard.
                // `the_repair_arm_reruns_the_backfill_without_undoing_a_declassification`
                // is the behavioural check.
                //
                // `ensure_privacy_schema` first, for arm 19's reason: this arm must
                // not assume any earlier arm ran.
                Self::ensure_privacy_schema(pool).await?;
                let _ = Self::backfill_privacy_from_recorded_provenance(pool).await?;
            }
            _ => {
                anyhow::bail!("Unknown migration version: {}", version);
            }
        }

        Ok(())
    }

    /// Whether `table` exists. Sibling of [`Self::table_has_column`]: a
    /// `pragma_table_info` on a missing table returns zero rows, so the column
    /// check alone cannot tell "no such table" from "no such column", and the
    /// backfill's shape guards need both answers.
    async fn table_exists(pool: &Pool<Sqlite>, table: &str) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name = ?1)",
        )
        .bind(table)
        .fetch_one(pool)
        .await?;
        Ok(exists)
    }

    async fn table_has_column(pool: &Pool<Sqlite>, table: &str, column: &str) -> Result<bool> {
        let query = format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1");
        let count: i64 = sqlx::query_scalar(&query)
            .bind(column)
            .fetch_one(pool)
            .await?;
        Ok(count > 0)
    }

    async fn ensure_message_identity_schema(pool: &Pool<Sqlite>) -> Result<()> {
        if !Self::table_has_column(pool, "messages", "msg_uid").await? {
            sqlx::query("ALTER TABLE messages ADD COLUMN msg_uid TEXT")
                .execute(pool)
                .await?;
        }

        sqlx::query("UPDATE messages SET msg_uid = 'm' || id WHERE msg_uid IS NULL")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_uid ON messages(session_id, msg_uid)",
        )
        .execute(pool)
        .await?;

        if !Self::table_has_column(pool, "sessions", "branch_point_msg_uid").await? {
            sqlx::query("ALTER TABLE sessions ADD COLUMN branch_point_msg_uid TEXT")
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    /// #51 W3: give every session ROW a token that is never handed to a later
    /// row with the same id, so a [`ConversationRevision`] taken from one
    /// incarnation can never be satisfied by another.
    ///
    /// Idempotent and version-independent, like the rest of
    /// `reconcile_loop_schema`. `ALTER TABLE ... ADD COLUMN` may not carry an
    /// expression default, so the column lands as `0` ("unknown") and is
    /// backfilled here; `random()` is re-evaluated per row, so the UPDATE gives
    /// each existing session its own value.
    async fn ensure_session_incarnation_schema(pool: &Pool<Sqlite>) -> Result<()> {
        if !Self::table_has_column(pool, "sessions", "incarnation").await? {
            sqlx::query("ALTER TABLE sessions ADD COLUMN incarnation INTEGER NOT NULL DEFAULT 0")
                .execute(pool)
                .await?;
        }
        sqlx::query("UPDATE sessions SET incarnation = random() WHERE IFNULL(incarnation, 0) = 0")
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Issue #56's columns, added idempotently and **version-independently**.
    ///
    /// The precedent is [`Self::ensure_session_incarnation_schema`], and the
    /// reason is the one `run_migrations` already records above the reconcile
    /// calls: development builds have shipped overlapping migration numbers
    /// before. Here it was not hypothetical — `feat/br71-workspace-control`
    /// claimed `CURRENT_SCHEMA_VERSION = 17` with its own `17 =>` arm adding
    /// `parent_session_id` while this work was still calling that number its
    /// own, and it landed first. With this helper the arm number stops being
    /// load-bearing and either merge order is safe.
    ///
    /// No backfill lives here. The backfill runs from the numbered arms and from
    /// the fresh-database import, never per launch: a startup-repeating `WHERE
    /// provider_name IN (..)` would re-privatise a session the user has just
    /// declassified, because declassification deliberately leaves
    /// `provider_name` untouched. That statement now also carries its own
    /// declassification guard — which makes a re-run non-destructive and is
    /// exactly why it must still not live here, since one guard between a
    /// per-launch rewrite of every row and the user's declassifications is one
    /// mechanism too few.
    async fn ensure_privacy_schema(pool: &Pool<Sqlite>) -> Result<()> {
        // BEGIN IMMEDIATE serializes the check-then-ALTER sequence across
        // concurrently running Biorouter processes, exactly as
        // `reconcile_usage_schema` does above. Without it two processes can both
        // observe the same missing column before either adds it, and the loser
        // aborts startup with `duplicate column name` — which is a migration
        // failure, not a retryable one. The desktop daemon, a terminal
        // `biorouter` and a scheduled job share this database, and the first
        // launch after an upgrade is when they are likeliest to start together.
        let mut connection = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        let result = Self::ensure_privacy_schema_locked(&mut connection).await;
        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *connection).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                Err(error)
            }
        }
    }

    async fn ensure_privacy_schema_locked(connection: &mut sqlx::SqliteConnection) -> Result<()> {
        for (column, sql_type) in [
            ("privacy_tier", "TEXT NOT NULL DEFAULT 'public'"),
            ("privacy_reason", "TEXT"),
            ("parent_session_id", "TEXT"),
            // Issue #56 DR-26 / Task 50 Step 3. A JSON array of institution ids
            // — the union of the institutions whose extensions this chat has
            // touched. NULL on every row that predates it, which reads as the
            // empty set: the same Missing-is-permissive direction the knowledge
            // store's affiliations map takes, and the same one the tier
            // migration took (AR-2).
            ("session_affiliations", "TEXT"),
        ] {
            let exists: i32 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?1",
            )
            .bind(column)
            .fetch_one(&mut *connection)
            .await?;
            if exists == 0 {
                sqlx::query(&format!(
                    "ALTER TABLE sessions ADD COLUMN {column} {sql_type}"
                ))
                .execute(&mut *connection)
                .await?;
            }
        }

        sqlx::query(CLASSIFICATION_AUDIT_DDL)
            .execute(&mut *connection)
            .await?;
        // Task 49's grant store, in the same idempotent, version-independent,
        // BEGIN IMMEDIATE-serialised place as the ledger above. A database that
        // already stands at `CURRENT_SCHEMA_VERSION` reaches this and no numbered
        // arm, which is the whole reason the table lives here.
        sqlx::query(CROSS_AFFILIATION_GRANTS_DDL)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    /// The providers whose bound sessions the one-time backfill privatises.
    ///
    /// A **literal**, not a lookup through `providers::providers()`, and the
    /// reason is not convenience. A migration classifies rows written by other
    /// builds: a database carrying `versa_bedrock` sessions can be opened by a
    /// binary compiled without the `aws-providers` feature, where that provider
    /// is absent from the live registry entirely and a registry-derived list
    /// would silently leave those rows public. The migration must also run
    /// before any provider is constructed, and must not depend on the user's
    /// config being readable.
    ///
    /// `the_backfilled_provider_set_is_every_provider_that_claims_private` is
    /// what stops the two drifting apart.
    const BACKFILL_PRIVATE_PROVIDERS: [&'static str; 4] =
        ["llamacpp", "ollama", "versa_azure", "versa_bedrock"];

    /// The backfill's `UPDATE`, composed rather than written out — and split out
    /// here so that the one composed tier assignment in the tree has a name a
    /// test can point at.
    ///
    /// ⚠ **Composition is how a tier assignment hides from the audits.**
    /// `exactly_one_statement_in_the_tree_assigns_a_public_classification` (in
    /// `privacy/declassify.rs`) matches one literal spelling, line by line, and
    /// says of itself that a statement built from variables is invisible to it.
    /// This is the tree's first runtime-composed `SET privacy_tier`, so it is the
    /// first thing shaped like that bypass — benign, because it interpolates
    /// [`SessionClassification::PRIVATE_SQL`] and raises rather than lowers, but
    /// the shape is now precedent. `the_backfill_statement_raises_and_nothing_else`
    /// pins the emitted text so the next edit to it has to be deliberate.
    ///
    /// It is composed at all because the provider list must stay a single source
    /// of truth with [`Self::BACKFILL_PRIVATE_PROVIDERS`], which
    /// `the_backfilled_provider_set_is_every_provider_that_claims_private` checks
    /// against the shipped provider modules.
    fn backfill_update_sql() -> String {
        format!(
            "UPDATE sessions \
                SET privacy_tier = '{private}', \
                    privacy_reason = 'backfill:' || provider_name \
              WHERE provider_name IN ({list}) \
                AND privacy_tier = '{public}' \
                AND {not_declassified}",
            private = SessionClassification::PRIVATE_SQL,
            public = SessionClassification::PUBLIC_SQL,
            list = Self::quoted_private_providers(),
            not_declassified = Self::NOT_DECLASSIFIED_BY_USER,
        )
    }

    /// [`Self::BACKFILL_PRIVATE_PROVIDERS`] as a SQL `IN` list, shared by the two
    /// raising statements so a provider can never be in one and not the other.
    fn quoted_private_providers() -> String {
        Self::BACKFILL_PRIVATE_PROVIDERS
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The declassification exclusion, as a correlated `NOT EXISTS` over the
    /// append-only ledger.
    ///
    /// ⚠ **`AND privacy_tier = 'public'` is NOT this check, and reading it as
    /// though it were is the bug that made the backfill a one-shot statement.**
    /// A declassified row IS public — `declassify` lowers the tier and
    /// deliberately leaves `provider_name` in place, because a public chat is
    /// allowed to run a private model. So the tier guard admits exactly the rows
    /// the user has just rescued, and a second run of the raising statement would
    /// silently undo the one irreversible action the design gives them.
    ///
    /// Keyed on the **ledger**, not on `privacy_reason = 'declassified_by_user'`.
    /// The reason column holds the *current* provenance and is overwritten by the
    /// next raise, so a chat that was declassified and later re-raised by a real
    /// turn no longer records the declassification anywhere else; the ledger row
    /// is append-only and permanent. It is also the artefact §12.5 already
    /// guarantees exists for every declassification, so this cannot drift from
    /// what "the user declassified it" means.
    ///
    /// The consequence is the point: with this clause the backfill is safe to run
    /// more than once, which is what lets arm 20 repair arm 19's rows at all.
    /// `the_repair_arm_reruns_the_backfill_without_undoing_a_declassification`
    /// is the behavioural proof, and it fails if this clause is deleted.
    const NOT_DECLASSIFIED_BY_USER: &str = "NOT EXISTS (SELECT 1 FROM classification_audit \
         ca WHERE ca.session_id = sessions.id AND ca.to_classification = 'public')";

    /// The **turn-ledger** raise — issue #56 finding 9.
    ///
    /// ⚠ **`sessions.provider_name` is the last binding, not the history**, and
    /// that residual used to be documented and left. The innocent path it costs:
    /// a user runs a chat on Ollama, then switches to Claude for one formatting
    /// question. `provider_name` reads `anthropic`, and the chat backfills
    /// **public** with a private transcript sitting in it.
    ///
    /// `token_events` is the fix, and it is not a transcript scan — the residual
    /// the design refused. It is a per-turn ledger the app has written since
    /// migration 11: one append-only row per billed turn, carrying the
    /// `session_id` and the `provider` that served it, from the same
    /// `Provider::get_name()` string that `sessions.provider_name` is bound from.
    /// A row there is the same fact the live ratchet fires on — *this session ran
    /// a turn against this provider* — recorded before the ratchet existed.
    ///
    /// **The reason is `turn:<provider>`, not `backfill:<provider>`, and that is
    /// a deliberate downgrade of the declassification control.** §12.4 grades on
    /// what actually happened: `backfill:*` takes the strong (typed-phrase +
    /// OS-password) control because the migration inferred a tier from a binding
    /// and observed nothing, and `strong_confirmation_reason` says exactly that to
    /// the user in those words. That sentence is FALSE for these rows — the ledger
    /// is an observation of a turn, which is precisely what `turn:*` means and
    /// what the live ratchet writes. Stamping `backfill:` here would show every
    /// one of these users a reason that did not happen, which is the specific
    /// falsehood `strong_confirmation_reason` exists to have stopped. And the
    /// comparison that matters is not against `backfill:` but against the status
    /// quo, which is **public with no control at all**: single-click-private is
    /// strictly more protection than the row has today.
    ///
    /// The named provider is the EARLIEST *trustworthy* private-tier one in the
    /// ledger, which is the turn that should have ratcheted the row had the
    /// feature existed then.
    ///
    /// Deliberately a second statement rather than an `OR` on the first: the two
    /// carry different provenance and different confirmation grades, and the
    /// ordering (last-bound first) is what keeps [`BackfillCounts::private`] and
    /// [`BackfillCounts::private_from_turn_history`] disjoint.
    fn backfill_turn_history_update_sql() -> String {
        let evidence = Self::TRUSTWORTHY_TURN_PROVIDER;
        let list = Self::quoted_private_providers();
        format!(
            "UPDATE sessions \
                SET privacy_tier = '{private}', \
                    privacy_reason = 'turn:' || ( \
                        SELECT te.provider FROM token_events te \
                         WHERE te.session_id = sessions.id \
                           AND te.provider IN ({list}) \
                           AND {evidence} \
                         ORDER BY te.ts ASC, te.id ASC LIMIT 1) \
              WHERE privacy_tier = '{public}' \
                AND EXISTS (SELECT 1 FROM token_events te \
                             WHERE te.session_id = sessions.id \
                               AND te.provider IN ({list}) \
                               AND {evidence}) \
                AND {not_declassified}",
            private = SessionClassification::PRIVATE_SQL,
            public = SessionClassification::PUBLIC_SQL,
            not_declassified = Self::NOT_DECLASSIFIED_BY_USER,
        )
    }

    /// Which `token_events` rows may be read as evidence of *which provider
    /// served this turn*.
    ///
    /// ⚠ **Not every row's `provider` describes the call it sits on, and this is
    /// the tree's own ruling rather than a new one.**
    /// `reconcile_usage_schema_locked` runs on every startup and NULLs
    /// `provider`/`model_id` on exactly the rows this predicate excludes, with the
    /// reason written beside it: the old v11/v12 development migrations copied
    /// each session's **final** model backward across its whole history, so
    /// without a durable event identity (`event_key`) or a billed total there is
    /// no evidence those values describe the original call.
    ///
    /// That is the SAME defect as finding 9 — a per-session last-provider value
    /// masquerading as history — one table over. Reading those rows would have
    /// re-imported the bug into its own fix, and on a first upgrade it would
    /// actually happen: `run_migrations` walks the numbered arms **before** it
    /// calls the reconcile, so on that one run the untrustworthy values are still
    /// on disk when the backfill looks.
    ///
    /// Found by `the_repair_arm_reruns_the_backfill_without_undoing_a_declassification`
    /// reporting one skipped row where two were expected — the fixture's own
    /// ledger row had been scrubbed by the reconcile between the arm and the
    /// assertion. `an_untrustworthy_ledger_row_does_not_classify_anything` is the
    /// direct test.
    const TRUSTWORTHY_TURN_PROVIDER: &str =
        "(te.event_key IS NOT NULL OR te.billed_total_tokens IS NOT NULL)";

    /// How many rows BOTH raising statements would have raised but skipped
    /// because the user declassified them.
    ///
    /// Reporting only — but it is the number that distinguishes "the guard is
    /// working" from "the guard matched nothing", and those are the same zero
    /// everywhere else.
    fn declassified_skipped_sql() -> String {
        let evidence = Self::TRUSTWORTHY_TURN_PROVIDER;
        let list = Self::quoted_private_providers();
        format!(
            "SELECT COUNT(*) FROM sessions \
              WHERE privacy_tier = '{public}' \
                AND (provider_name IN ({list}) \
                     OR EXISTS (SELECT 1 FROM token_events te \
                                 WHERE te.session_id = sessions.id \
                                   AND te.provider IN ({list}) \
                                   AND {evidence})) \
                AND NOT ({not_declassified})",
            public = SessionClassification::PUBLIC_SQL,
            not_declassified = Self::NOT_DECLASSIFIED_BY_USER,
        )
    }

    /// Issue #56 §15 — the classification backfill, from every provenance the
    /// database actually records.
    ///
    /// ⚠ **It belongs to the numbered migration arms and to the fresh-database
    /// import, and to nothing else.** [`Self::ensure_privacy_schema`] runs on
    /// **every** startup, and that remains the wrong home even now that the
    /// statements are declassification-guarded: the guard makes a re-run
    /// *non-destructive*, it does not make a per-launch re-scan of every row a
    /// thing this code should do, and one guard standing between a per-startup
    /// statement and the user's declassifications is one mechanism too few.
    /// `the_backfill_runs_from_the_migration_arms_and_the_import_and_nowhere_else`
    /// pins the call sites.
    ///
    /// **Two evidence sources, in this order.**
    ///
    /// 1. [`Self::backfill_update_sql`] — the row's bound provider. What issue
    ///    #56 shipped, unchanged apart from the declassification guard.
    /// 2. [`Self::backfill_turn_history_update_sql`] — the `token_events` turn
    ///    ledger. This is finding 9's fix: `provider_name` is the LAST binding,
    ///    so a chat that ran on Ollama and was later switched to Claude read
    ///    `anthropic` and backfilled public with a private transcript. The ledger
    ///    still holds its Ollama turns. See that function for why those rows are
    ///    stamped `turn:` rather than `backfill:`.
    ///
    /// The order is load-bearing for the counts, not for the outcome: the second
    /// statement's `AND privacy_tier = 'public'` means it can only see rows the
    /// first left alone, which is what makes the two counts disjoint.
    ///
    /// Still fails OPEN where it has nothing (DR-10). A fail-CLOSED backfill
    /// (NULL provider plus at least one message ⇒ private) was rejected: a user
    /// who has only ever used a commercial provider would find a large slice of
    /// their history marked private on first launch, refused on the model they
    /// normally use, with only an irreversible declassification as the exit, one
    /// chat at a time.
    ///
    /// The residual, narrower than it was but still real: a session whose private
    /// turns predate `token_events.provider` (migration 11) and which was later
    /// rebound to a public provider records the private work in neither column,
    /// and backfills public. There is no transcript scan and there will not be
    /// one. `docs/security/privacy-tiers-migration.md` says this to the user.
    ///
    /// `AND privacy_tier = 'public'` is not redundant: a database that reached an
    /// arm with the columns already present (BR-71's number collision is exactly
    /// that case) can hold rows a running build already raised, and the ratchet
    /// must never be walked backwards or re-stamped with a weaker provenance.
    async fn backfill_privacy_from_recorded_provenance(
        pool: &Pool<Sqlite>,
    ) -> Result<BackfillCounts> {
        // Shape-guarded for the same reason `ensure_privacy_schema` is: this arm
        // must not assume an earlier arm ran. `provider_name` arrives in
        // migration 6, so every database that walked the ladder has it — but a
        // database that reaches here without it records no provider for any row,
        // which is precisely the "unknown, so fail open" case. Backfilling
        // nothing is the correct answer; aborting startup with `no such column`
        // is not. `experimental_loop_v11_through_v14_shapes_upgrade_without_loss`
        // and `pr13_v12_database_reconciles_usage_and_adds_loop_schema` are the
        // two shapes that reach this branch.
        if !Self::table_has_column(pool, "sessions", "provider_name").await? {
            warn!(
                "issue #56: skipping the privacy backfill; this database's `sessions` \
                 table has no `provider_name`, so no row's tier can be inferred"
            );
            return Ok(BackfillCounts::default());
        }

        // ⚠ The declassification guard reads `classification_audit`, so BOTH
        // statements below name a table that a database arriving from an early
        // enough version has never had. `no such table` there is a failed
        // startup, and — worse — the obvious repair (drop the guard when the
        // table is missing) is the un-declassification bug wearing a fallback.
        // Creating it is unconditionally correct instead: the DDL is
        // `IF NOT EXISTS`, `ensure_privacy_schema` runs the identical statement
        // on every launch anyway, and a database with no ledger has by
        // construction recorded no declassification for the guard to miss.
        sqlx::query(CLASSIFICATION_AUDIT_DDL).execute(pool).await?;

        // The turn ledger is older still (`token_events` at migration 9, its
        // `provider` column at 11, `event_key`/`billed_total_tokens` at 12-14),
        // but a database that reaches here without it must lose the second
        // evidence source, not the startup. All four columns are required, not
        // just `provider`: without the two identity columns
        // `TRUSTWORTHY_TURN_PROVIDER` cannot be evaluated, and dropping the
        // predicate instead would read exactly the backward-copied values it
        // exists to reject.
        let turn_ledger = Self::table_exists(pool, "token_events").await?
            && Self::table_has_column(pool, "token_events", "provider").await?
            && Self::table_has_column(pool, "token_events", "event_key").await?
            && Self::table_has_column(pool, "token_events", "billed_total_tokens").await?;
        if !turn_ledger {
            warn!(
                "issue #56: this database has no usable turn ledger, so the backfill reads \
                 only each row's bound provider, so chats that switched providers may stay public"
            );
        }

        let private = sqlx::query(&Self::backfill_update_sql())
            .execute(pool)
            .await?
            .rows_affected() as i64;

        let private_from_turn_history = if turn_ledger {
            sqlx::query(&Self::backfill_turn_history_update_sql())
                .execute(pool)
                .await?
                .rows_affected() as i64
        } else {
            0
        };

        // ── Everything below this line is REPORTING, and none of it may fail the
        // migration. ────────────────────────────────────────────────────────────
        //
        // ⚠ The `UPDATE` has already committed by the time these run, but the
        // version counter has not: `run_migrations` calls `update_schema_version`
        // only after `apply_migration` returns `Ok`. So a `?` on a COUNT would
        // leave the database backfilled and still numbered one below the arm —
        // and the next launch would re-enter this arm and re-run a statement
        // whose whole contract is that it runs once. The `AND privacy_tier =
        // 'public'` guard means the re-run is not destructive today, but the
        // one-shot property must not rest on a second mechanism, and none of
        // these numbers is worth a failed startup: they feed a log line that
        // nothing branches on.
        let count = |sql: String| async move {
            match sqlx::query_scalar::<_, i64>(&sql).fetch_one(pool).await {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        %error,
                        "issue #56: a privacy-backfill report query failed; the backfill \
                         itself already committed, so this is logged and not raised"
                    );
                    -1
                }
            }
        };

        // What the migration did, in the buckets a support conversation actually
        // needs. `public_named`/`unknown_provider` plus the private rows partition
        // the table; `backfilled_empty` cuts across it and is the gap between
        // these numbers and the ones the user can see, since History hides
        // message-less rows. `-1` in any of them means "the count failed", which
        // is why they are `i64` and not `u64`.
        let public_named = count(
            "SELECT COUNT(*) FROM sessions \
              WHERE IFNULL(privacy_tier, '') = 'public' AND IFNULL(provider_name, '') <> ''"
                .to_string(),
        )
        .await;
        let unknown_provider = count(
            "SELECT COUNT(*) FROM sessions \
              WHERE IFNULL(privacy_tier, '') = 'public' AND IFNULL(provider_name, '') = ''"
                .to_string(),
        )
        .await;
        let empty = count(
            "SELECT COUNT(*) FROM sessions s \
              WHERE NOT EXISTS (SELECT 1 FROM messages m WHERE m.session_id = s.id)"
                .to_string(),
        )
        .await;
        // Rows the evidence pointed at and the guard held back. Zero on a machine
        // that has never declassified anything, which is most of them — its value
        // is that a NON-zero here is the only place the guard is visible at all.
        let declassified_skipped = if turn_ledger {
            count(Self::declassified_skipped_sql()).await
        } else {
            0
        };

        let visible = Self::privacy_notice_counts(pool)
            .await
            .unwrap_or_else(|error| {
                warn!(
                    %error,
                    "issue #56: could not compute the day-one notice counts for the \
                     migration log; the renderer computes its own from the session list"
                );
                PrivacyNoticeCounts::default()
            });

        info!(
            backfilled_private = private,
            backfilled_private_from_turn_history = private_from_turn_history,
            backfilled_declassified_skipped = declassified_skipped,
            backfilled_public_named = public_named,
            backfilled_unknown_provider = unknown_provider,
            backfilled_empty = empty,
            notice_private_visible = visible.private_visible,
            notice_public_named_visible = visible.public_named_visible,
            notice_unknown_provider_visible = visible.unknown_provider_visible,
            notice_total_visible = visible.total_visible,
            "issue #56: privacy backfill from each session's bound provider and turn ledger"
        );
        Ok(BackfillCounts {
            private,
            private_from_turn_history,
            declassified_skipped,
            public_named,
            unknown_provider,
            empty,
        })
    }

    /// Walk the recorded schema version back below the privacy repair arm, so the
    /// next launch re-enters it.
    ///
    /// The one caller is the fresh-database import path, where `create_schema`
    /// has already stamped `CURRENT_SCHEMA_VERSION` and a failed backfill would
    /// otherwise strand imported chats public on a database that reports itself
    /// fully migrated — no arm re-runs, and the failure is a log line nobody
    /// reads. Walking the counter back converts it into a retry.
    ///
    /// Safe because arm 20 is `ensure_privacy_schema` + this backfill, and both
    /// are idempotent: the columns are shape-guarded and the raising statements
    /// are `AND privacy_tier = 'public'` plus the declassification guard. Deleting
    /// only versions at or above the repair arm keeps every lower arm's record, so
    /// no earlier migration re-runs.
    ///
    /// Best-effort by construction — it is already the recovery path, and a
    /// failure here leaves exactly the state that not calling it would.
    ///
    /// ⚠ **The DELETE alone is not the rewind, and a version of this that stopped
    /// there was written and caught by
    /// `the_retreat_moves_the_counter_back_into_the_repair_arm`.** `create_schema`
    /// records exactly ONE row, so deleting everything at or above the repair arm
    /// empties the table — and `get_schema_version` then reads `MAX(version)` over
    /// no rows and reports **0**. The next launch would replay the entire ladder
    /// from migration 1 against a database that already has every table, which
    /// aborts startup on `table … already exists`. Turning a mis-classification
    /// into an unopenable database is not a recovery. The row below is what makes
    /// the counter land ON the arm's predecessor instead of at the bottom.
    async fn retreat_to_privacy_repair(pool: &Pool<Sqlite>) {
        let below = PRIVACY_REPAIR_ARM - 1;
        let rewind = async {
            sqlx::query("DELETE FROM schema_version WHERE version >= ?1")
                .bind(PRIVACY_REPAIR_ARM)
                .execute(pool)
                .await?;
            // `OR IGNORE` because `version` is the primary key and a database
            // that walked the ladder already records this number.
            sqlx::query("INSERT OR IGNORE INTO schema_version (version) VALUES (?1)")
                .bind(below)
                .execute(pool)
                .await?;
            Ok::<(), sqlx::Error>(())
        };
        match rewind.await {
            Ok(()) => warn!(
                "issue #56: rewound the schema counter to v{below}; the next launch will \
                 re-enter the privacy repair arm and retry the backfill"
            ),
            Err(error) => warn!(
                %error,
                "issue #56: could not rewind the schema counter; imported legacy chats may \
                 stay classified public"
            ),
        }
    }

    /// The day-one notice's numbers (§15.5), computed from the user's own
    /// database — never hardcoded, because the measured figures moved by a
    /// factor of three in four days while this was being designed.
    ///
    /// The population is History's: `session_type IN ('user','scheduled')` with
    /// at least one message. `EXISTS` rather than
    /// `list_sessions_by_types`' `INNER JOIN … GROUP BY`, which selects the same
    /// rows but has to materialise every session to count them.
    ///
    /// `IFNULL(privacy_tier, '') <> 'public'` mirrors
    /// [`SessionClassification::from_stored`]: anything that is not exactly
    /// `public` reads Private, so the notice counts what the badges in History
    /// will actually show rather than what a permissive parse would.
    pub(crate) async fn privacy_notice_counts(pool: &Pool<Sqlite>) -> Result<PrivacyNoticeCounts> {
        let (private_visible, public_named_visible, unknown_provider_visible, total_visible) =
            sqlx::query_as::<_, (i64, i64, i64, i64)>(
                // COALESCE because SUM over zero rows is NULL, not 0 — a fresh
                // install would otherwise fail to decode into `i64` and take
                // the migration down with it.
                "SELECT
                    COALESCE(SUM(CASE WHEN IFNULL(s.privacy_tier, '') <> 'public'
                                      THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN IFNULL(s.privacy_tier, '') = 'public'
                                       AND IFNULL(s.provider_name, '') <> ''
                                      THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN IFNULL(s.privacy_tier, '') = 'public'
                                       AND IFNULL(s.provider_name, '') = ''
                                      THEN 1 ELSE 0 END), 0),
                    COUNT(*)
                   FROM sessions s
                  WHERE s.session_type IN ('user', 'scheduled')
                    AND EXISTS (SELECT 1 FROM messages m WHERE m.session_id = s.id)",
            )
            .fetch_one(pool)
            .await?;

        Ok(PrivacyNoticeCounts {
            private_visible,
            public_named_visible,
            unknown_provider_visible,
            total_visible,
        })
    }

    /// The append-only declassification ledger (§12.5).
    ///
    /// Shared by `create_schema` and [`Self::ensure_privacy_schema`] rather than
    /// living only in the reconcile, because the two schema paths are mutually
    /// exclusive: a database with no `schema_version` table takes `create_schema`
    /// and never reaches `run_migrations` in that process. Creating the table
    /// only in the reconcile would leave a first-run install without it until
    /// the *second* launch — and a declassification in between fails with
    /// `no such table`.
    async fn create_classification_audit_table(pool: &Pool<Sqlite>) -> Result<()> {
        sqlx::query(CLASSIFICATION_AUDIT_DDL).execute(pool).await?;
        Ok(())
    }

    async fn create_and_backfill_messages_fts(pool: &Pool<Sqlite>, rebuild: bool) -> Result<()> {
        let existed = Self::messages_fts_exists(pool).await;
        sqlx::query(MESSAGES_FTS_DDL).execute(pool).await?;
        if existed && !rebuild {
            return Ok(());
        }

        sqlx::query("DELETE FROM messages_fts")
            .execute(pool)
            .await?;
        let rows = sqlx::query_as::<_, (i64, String, String, Option<String>)>(
            "SELECT id, session_id, content_json, metadata_json FROM messages",
        )
        .fetch_all(pool)
        .await?;

        for (id, session_id, content_json, metadata_json) in rows {
            if !message_is_user_visible(metadata_json.as_deref()) {
                continue;
            }
            let Ok(content) = serde_json::from_str::<Vec<MessageContent>>(&content_json) else {
                continue;
            };
            let text = chat_fts::extract_searchable_text(&content);
            if text.is_empty() {
                continue;
            }
            sqlx::query(MESSAGES_FTS_INSERT)
                .bind(&text)
                .bind(&session_id)
                .bind(id)
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    /// The BR-43 `checkpoints` side table (migration 13 + fresh-DB schema).
    async fn create_checkpoints_table(pool: &Pool<Sqlite>) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_index INTEGER NOT NULL,
                anchor_ts INTEGER NOT NULL,
                kind TEXT NOT NULL,
                commit_sha TEXT NOT NULL,
                tree_sha TEXT NOT NULL,
                changed_paths_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
        "#,
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_session ON checkpoints(session_id, turn_index)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// BR-7: side table holding tool-result payloads too large to keep inline in
    /// `messages.content_json`. Keyed `(session_id, blob_uid)` rather than by the
    /// message rowid: `replace_conversation_inner` DELETEs and re-INSERTs every
    /// message on each compaction/edit, so a rowid reference would dangle on the
    /// first rewrite. The composite key also lets a diverged/copied session own
    /// its own row for the same payload, so the parent's orphan sweep can never
    /// pull a blob out from under a branch.
    async fn create_message_blobs_table(pool: &Pool<Sqlite>) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS message_blobs (
                blob_uid TEXT NOT NULL,
                session_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                bytes INTEGER NOT NULL,
                content TEXT NOT NULL,
                PRIMARY KEY (session_id, blob_uid)
            )
        "#,
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_message_blobs_uid ON message_blobs(blob_uid)")
            .execute(pool)
            .await?;
        Ok(())
    }

    /// The 8-character prefix a new session id is built on.
    ///
    /// Production uses today's date, which is what makes ids sort and read
    /// sensibly. **Tests must not**, and the reason is worth stating because the
    /// symptom is nothing like the cause: ids are minted as `PREFIX_N` where `N`
    /// is `MAX(N) + 1` *within this database*, and every test builds its own
    /// `TempDir` manager over an EMPTY one. So the first session of every test
    /// is `20260827_1`, the second `20260827_2`, and so on — ids collide across
    /// tests by construction.
    ///
    /// That matters because `agents::subagent_handle::HANDLES` is process-global
    /// and keyed by session id. A test that leaves a handle behind hands it to
    /// whichever later test mints the same id, whose `Agent::reply` then waits
    /// on a handle belonging to a test that finished long ago. That is what hung
    /// `test (macos-latest)` and `test (windows-latest)` until the job timed out,
    /// discarding ~3000 passing results, and nothing in the failure named it.
    ///
    /// ⚠ The prefix MUST stay 8 characters. `create_session` reads the counter
    /// back with `SUBSTR(id, 10)`, which assumes 8 + the underscore.
    ///
    /// Derived from the store's own directory, so it is stable for one manager
    /// (the counter still increments correctly) and distinct between managers
    /// (each test has its own `TempDir`).
    #[cfg(test)]
    fn id_prefix(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.session_dir.hash(&mut hasher);
        format!("{:08x}", hasher.finish() as u32)
    }

    #[cfg(not(test))]
    fn id_prefix(&self) -> String {
        chrono::Utc::now().format("%Y%m%d").to_string()
    }

    async fn create_session(
        &self,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
    ) -> Result<Session> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;

        let today = self.id_prefix();
        let session = sqlx::query_as(
            r#"
                INSERT INTO sessions (id, name, user_set_name, session_type, working_dir, extension_data, incarnation)
                VALUES (
                    ? || '_' || CAST(COALESCE((
                        SELECT MAX(CAST(SUBSTR(id, 10) AS INTEGER))
                        FROM sessions
                        WHERE id LIKE ? || '_%'
                    ), 0) + 1 AS TEXT),
                    ?,
                    FALSE,
                    ?,
                    ?,
                    '{}',
                    random()
                )
                RETURNING *
                "#,
        )
            .bind(&today)
            .bind(&today)
            .bind(&name)
            .bind(session_type.to_string())
            .bind(working_dir.to_string_lossy().as_ref())
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(session)
    }

    /// Resume the session bound to `external_key`, or create a fresh one and bind
    /// it. Returns `(session, resumed)`. The session's real primary key remains
    /// the allocated `YYYYMMDD_N` id; `external_key` is only a stable lookup
    /// handle for durable, resumable app sessions.
    async fn get_or_create_by_external_key(
        &self,
        external_key: &str,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
    ) -> Result<(Session, bool)> {
        let pool = self.pool().await?;
        if let Some(id) =
            sqlx::query_scalar::<_, String>("SELECT id FROM sessions WHERE external_key = ?")
                .bind(external_key)
                .fetch_optional(pool)
                .await?
        {
            return Ok((self.get_session(&id, true).await?, true));
        }

        let session = self.create_session(working_dir, name, session_type).await?;
        match sqlx::query("UPDATE sessions SET external_key = ? WHERE id = ?")
            .bind(external_key)
            .bind(&session.id)
            .execute(pool)
            .await
        {
            Ok(_) => Ok((session, false)),
            // ONLY a UNIQUE-constraint violation means another connection bound
            // this key first (a genuine lost race) — recover by discarding our
            // duplicate and resuming the winner. Any OTHER error (SQLITE_BUSY,
            // I/O, disk-full, …) is transient/retryable and must NOT destroy our
            // freshly-created session, so propagate it unchanged.
            Err(e)
                if matches!(
                    e.as_database_error().map(|d| d.kind()),
                    Some(sqlx::error::ErrorKind::UniqueViolation)
                ) =>
            {
                let _ = sqlx::query("DELETE FROM sessions WHERE id = ?")
                    .bind(&session.id)
                    .execute(pool)
                    .await;
                let id = sqlx::query_scalar::<_, String>(
                    "SELECT id FROM sessions WHERE external_key = ?",
                )
                .bind(external_key)
                .fetch_one(pool)
                .await?;
                Ok((self.get_session(&id, true).await?, true))
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn get_session(&self, id: &str, include_messages: bool) -> Result<Session> {
        let pool = self.pool().await?;
        let mut session = sqlx::query_as::<_, Session>(
            r#"
        SELECT id, working_dir, name, description, user_set_name, session_type, created_at, updated_at, extension_data,
               total_tokens, input_tokens, output_tokens,
               accumulated_total_tokens, accumulated_input_tokens, accumulated_output_tokens,
               schedule_id, workflow_json, user_workflow_values_json,
               provider_name, model_config_json, diverged_from, branch_point_msg_uid, parent_session_id,
               privacy_tier, privacy_reason
        FROM sessions
        WHERE id = ?
    "#,
        )
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if include_messages {
            let conv = self.get_conversation(&session.id).await?;
            session.message_count = conv.messages().len();
            session.conversation = Some(conv);
        } else {
            let count =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE session_id = ?")
                    .bind(&session.id)
                    .fetch_one(pool)
                    .await? as usize;
            session.message_count = count;
        }

        Ok(session)
    }

    async fn get_token_counts(&self, id: &str) -> Result<SessionTokenCounts> {
        let pool = self.pool().await?;
        let counts = sqlx::query_as::<_, SessionTokenCounts>(
            r#"
        SELECT total_tokens, input_tokens, output_tokens,
               accumulated_total_tokens, accumulated_input_tokens, accumulated_output_tokens
        FROM sessions
        WHERE id = ?
    "#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        Ok(counts)
    }

    /// The atomic conditional update behind
    /// [`SessionManager::try_update_working_dir_if_empty`]: one `UPDATE` whose
    /// `WHERE` clause carries the emptiness check, so check and write cannot
    /// be interleaved by a concurrent message insert (SQLite serializes
    /// writers; the `NOT EXISTS` is evaluated within the same statement).
    async fn try_update_working_dir_if_empty(
        &self,
        id: &str,
        working_dir: &Path,
    ) -> Result<WorkingDirUpdate> {
        let pool = self.pool().await?;
        let result = sqlx::query(
            "UPDATE sessions SET working_dir = ?, updated_at = datetime('now') \
             WHERE id = ? AND NOT EXISTS (SELECT 1 FROM messages WHERE session_id = ?)",
        )
        .bind(working_dir.to_string_lossy().as_ref())
        .bind(id)
        .bind(id)
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(WorkingDirUpdate::Updated);
        }

        // 0 rows: either the session has messages or it does not exist.
        let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
        if exists > 0 {
            Ok(WorkingDirUpdate::RefusedNotEmpty)
        } else {
            Ok(WorkingDirUpdate::SessionNotFound)
        }
    }

    /// See [`SessionManager::get_extension_state`].
    async fn get_extension_state(
        &self,
        session_id: &str,
        extension: &str,
        version: &str,
    ) -> Result<Option<serde_json::Value>> {
        let pool = self.pool().await?;
        let Some(raw) = sqlx::query_scalar::<_, Option<String>>(
            "SELECT extension_data FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(pool)
        .await?
        .flatten() else {
            return Ok(None);
        };
        let data = Self::parse_extension_data(&raw)?;
        Ok(data
            .get_extension_state(extension, version)
            .filter(|v| !v.is_null())
            .cloned())
    }

    /// See [`SessionManager::update_extension_state`] — including why the first
    /// statement of the transaction must be a write.
    async fn update_extension_state<F>(
        &self,
        session_id: &str,
        extension: &str,
        version: &str,
        mutate: F,
    ) -> Result<Option<serde_json::Value>>
    where
        F: FnOnce(Option<&serde_json::Value>) -> Result<serde_json::Value> + Send,
    {
        self.update_extension_data(session_id, move |data| {
            let current = data
                .get_extension_state(extension, version)
                .filter(|v| !v.is_null())
                .cloned();
            let next = mutate(current.as_ref())?;
            data.set_extension_state(extension, version, next.clone());
            Ok(next)
        })
        .await
    }

    /// See [`SessionManager::sessions_mentioning_extension`].
    async fn sessions_mentioning_extension(&self, extension_name: &str) -> Result<Vec<String>> {
        // The name is matched against the blob as JSON WROTE it, not as the
        // caller spelled it: `validate_extension_name` refuses only path
        // separators, so a name may still contain a quote or a backslash, and
        // those reach the column escaped. Encoding through serde and dropping
        // the wrapping quotes is how the needle is guaranteed to be the same
        // bytes the column holds.
        let encoded = serde_json::to_string(extension_name)?;
        let needle = encoded
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(&encoded);
        let mut pattern = String::with_capacity(needle.len() + 2);
        pattern.push('%');
        for character in needle.chars() {
            if matches!(character, '%' | '_' | '\\') {
                pattern.push('\\');
            }
            pattern.push(character);
        }
        pattern.push('%');

        let pool = self.pool().await?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT id FROM sessions \
             WHERE session_type != 'sub_agent' \
               AND extension_data IS NOT NULL \
               AND extension_data LIKE ? ESCAPE '\\'",
        )
        .bind(pattern)
        .fetch_all(pool)
        .await?;
        Ok(ids)
    }

    async fn update_extension_data<F, R>(&self, session_id: &str, mutate: F) -> Result<Option<R>>
    where
        F: FnOnce(&mut ExtensionData) -> Result<R> + Send,
        R: Send,
    {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;

        // LOAD-BEARING FIRST STATEMENT, AND IT MUST BE A WRITE. DO NOT REORDER.
        // See `replace_conversation_inner` for the measurement behind this: a
        // deferred WAL transaction that reads first cannot upgrade to a writer
        // without an immediate `SQLITE_BUSY_SNAPSHOT`, and the busy timeout does
        // not apply. Taking the write lock up front makes the SELECT below read
        // latest-committed state, which is what makes the merge sound.
        //
        // It also keeps `updated_at` moving, exactly as the
        // `update(id).extension_data(..)` path this replaces did.
        let touched = sqlx::query("UPDATE sessions SET updated_at = datetime('now') WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        if touched.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }

        let raw = sqlx::query_scalar::<_, Option<String>>(
            "SELECT extension_data FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or_else(|| "{}".to_string());
        let mut data = match Self::parse_extension_data(&raw) {
            Ok(data) => data,
            Err(e) => {
                tx.rollback().await?;
                return Err(e);
            }
        };

        let result = match mutate(&mut data) {
            Ok(result) => result,
            Err(e) => {
                tx.rollback().await?;
                return Err(e);
            }
        };

        sqlx::query("UPDATE sessions SET extension_data = ? WHERE id = ?")
            .bind(serde_json::to_string(&data)?)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(result))
    }

    /// Strict, unlike the tolerant `unwrap_or_default()` in `Session::from_row`.
    /// A read-modify-write of one key rewrites the WHOLE column, so silently
    /// treating an unparseable blob as empty would erase every other
    /// extension's state; refusing to write is the only safe answer.
    fn parse_extension_data(raw: &str) -> Result<ExtensionData> {
        serde_json::from_str(raw)
            .map_err(|e| anyhow::anyhow!("session extension_data is not a JSON object: {e}"))
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_update(&self, builder: SessionUpdateBuilder<'_>) -> Result<()> {
        let mut updates = Vec::new();
        let mut query = String::from("UPDATE sessions SET ");

        macro_rules! add_update {
            ($field:expr, $name:expr) => {
                if $field.is_some() {
                    if !updates.is_empty() {
                        query.push_str(", ");
                    }
                    updates.push($name);
                    query.push_str($name);
                    query.push_str(" = ?");
                }
            };
        }

        add_update!(builder.name, "name");
        add_update!(builder.user_set_name, "user_set_name");
        add_update!(builder.session_type, "session_type");
        add_update!(builder.working_dir, "working_dir");
        add_update!(builder.extension_data, "extension_data");
        add_update!(builder.total_tokens, "total_tokens");
        add_update!(builder.input_tokens, "input_tokens");
        add_update!(builder.output_tokens, "output_tokens");
        add_update!(builder.accumulated_total_tokens, "accumulated_total_tokens");
        add_update!(builder.accumulated_input_tokens, "accumulated_input_tokens");
        add_update!(
            builder.accumulated_output_tokens,
            "accumulated_output_tokens"
        );

        // Additive, atomic accumulation. Emitted as `col = COALESCE(col,0) + ?`
        // so a concurrent turn on the same session cannot lose an update.
        if let Some(delta) = builder.token_delta {
            for (value, name) in [
                (delta.total, "accumulated_total_tokens"),
                (delta.input, "accumulated_input_tokens"),
                (delta.output, "accumulated_output_tokens"),
            ] {
                if value.is_none() {
                    continue;
                }
                if !updates.is_empty() {
                    query.push_str(", ");
                }
                updates.push(name);
                query.push_str(name);
                query.push_str(" = COALESCE(");
                query.push_str(name);
                query.push_str(", 0) + ?");
            }
        }

        add_update!(builder.schedule_id, "schedule_id");
        add_update!(builder.workflow, "workflow_json");
        add_update!(builder.user_workflow_values, "user_workflow_values_json");
        add_update!(builder.provider_name, "provider_name");
        add_update!(builder.model_config, "model_config_json");
        add_update!(builder.diverged_from, "diverged_from");
        add_update!(builder.branch_point_msg_uid, "branch_point_msg_uid");
        add_update!(builder.parent_session_id, "parent_session_id");

        // THE load-bearing line of issue #56. Emitted as a CASE so the storage
        // layer, not the caller, is what refuses a downgrade. Concurrency is
        // safe in both orderings. `privacy_reason` is guarded by the same
        // predicate, so a refused raise cannot rewrite the provenance the
        // declassification dialog grades on (§12.4). Both right-hand sides read
        // the row's pre-UPDATE `privacy_tier`, which is what makes the pair
        // agree with each other.
        //
        // The predicate is "the row is exactly `public`", NOT "the row is not
        // `private`", so that it is the same predicate
        // `SessionClassification::from_stored` reads with. That reader fails
        // closed: NULL, `PUBLIC` and anything else unrecognised all come back
        // Private. Keying the SQL on `= 'private'` instead would leave every one
        // of those rows — private to the whole Rust tree — assignable to a
        // canonical `public` by any caller, which is the reversal this fragment
        // exists to make impossible. A non-canonical value is preserved verbatim
        // rather than canonicalised, so the anomaly stays visible; it reads
        // Private either way.
        //
        // `privacy_reason` is not merely frozen alongside it: it accumulates by
        // DOMINANCE, `mcp:*` over everything else. §12.4 grades the
        // declassification confirmation on whether a private data source was
        // ever reached (`mcp:*` ⇒ typed confirmation) or whether the session only
        // ran a turn against a private endpoint (`turn:*` ⇒ single click with
        // undo). Gate B raises at reply entry and Gate C on dispatch, and Gate C
        // can only fire in a session already bound to a private provider — so
        // `turn:*` always lands first and every `mcp:` event arrives at a row
        // that is already private. A plainly frozen reason would hide every
        // `mcp:` event that has ever happened. Last-write-wins is equally wrong
        // in the other direction: the next ordinary turn would erase it. So an
        // `mcp:` raise displaces a non-`mcp:` provenance, and nothing displaces
        // an `mcp:` one. The vocabulary is `Session::privacy_reason`'s; SQLite's
        // LIKE is ASCII-case-insensitive, which only ever grades a session more
        // strictly.
        //
        // ⚠ Both dominance arms are guarded on the row being ALREADY non-public;
        // the trailing `ELSE ?` writes the incoming reason unconditionally. That
        // is right today, because on a public row there is no provenance worth
        // keeping. It stops being right the moment §12.5 declassification starts
        // leaving `declassified_by_user` in `privacy_reason` on a row it has just
        // returned to `public` — a later `raise_privacy(Public, ..)` would then
        // erase the record of the declassification, and
        // `every_projection_that_builds_a_session_reads_the_column` already
        // treats a reason on a public row as meaningful data. Whoever lands
        // §12.5 declassification has to decide here whether the `ELSE` arm
        // preserves a `declassified_by_user` provenance the way the `mcp:` arm
        // preserves its own.
        //
        // (This note used to name Gate B's task as one of the deciders. Gate B
        // has landed and correctly decided nothing: its turn ratchet raises a
        // PUBLIC row to private with `turn:<provider>`, which is precisely the
        // `ELSE` arm doing the right thing on a row with no provenance to keep.
        // The trigger is declassification existing, not this arm acquiring
        // another caller.)
        if builder.privacy_raise.is_some() {
            if !updates.is_empty() {
                query.push_str(", ");
            }
            updates.push("privacy_tier");
            query.push_str(
                "privacy_tier = CASE WHEN IFNULL(privacy_tier, '') <> 'public' \
                 THEN privacy_tier ELSE ? END, \
                 privacy_reason = CASE \
                 WHEN IFNULL(privacy_tier, '') <> 'public' \
                 AND IFNULL(privacy_reason, '') NOT LIKE 'mcp:%' \
                 AND ? LIKE 'mcp:%' THEN ? \
                 WHEN IFNULL(privacy_tier, '') <> 'public' THEN privacy_reason \
                 ELSE ? END",
            );
        }

        if updates.is_empty() {
            return Ok(());
        }

        query.push_str(", ");
        query.push_str("updated_at = datetime('now') WHERE id = ?");

        let mut q = sqlx::query(&query);

        if let Some(name) = builder.name {
            q = q.bind(name);
        }
        if let Some(user_set_name) = builder.user_set_name {
            q = q.bind(user_set_name);
        }
        if let Some(session_type) = builder.session_type {
            q = q.bind(session_type.to_string());
        }
        if let Some(wd) = builder.working_dir {
            q = q.bind(wd.to_string_lossy().to_string());
        }
        if let Some(ed) = builder.extension_data {
            q = q.bind(serde_json::to_string(&ed)?);
        }
        if let Some(tt) = builder.total_tokens {
            q = q.bind(tt);
        }
        if let Some(it) = builder.input_tokens {
            q = q.bind(it);
        }
        if let Some(ot) = builder.output_tokens {
            q = q.bind(ot);
        }
        if let Some(att) = builder.accumulated_total_tokens {
            q = q.bind(att);
        }
        if let Some(ait) = builder.accumulated_input_tokens {
            q = q.bind(ait);
        }
        if let Some(aot) = builder.accumulated_output_tokens {
            q = q.bind(aot);
        }
        if let Some(delta) = builder.token_delta {
            // Bind order must match the clause order appended above.
            for value in [delta.total, delta.input, delta.output]
                .into_iter()
                .flatten()
            {
                q = q.bind(value);
            }
        }
        if let Some(sid) = builder.schedule_id {
            q = q.bind(sid);
        }
        if let Some(workflow) = builder.workflow {
            let workflow_json = workflow.map(|r| serde_json::to_string(&r)).transpose()?;
            q = q.bind(workflow_json);
        }
        if let Some(user_workflow_values) = builder.user_workflow_values {
            let user_workflow_values_json = user_workflow_values
                .map(|urv| serde_json::to_string(&urv))
                .transpose()?;
            q = q.bind(user_workflow_values_json);
        }
        if let Some(provider_name) = builder.provider_name {
            q = q.bind(provider_name);
        }
        if let Some(model_config) = builder.model_config {
            let model_config_json = model_config
                .map(|mc| serde_json::to_string(&mc))
                .transpose()?;
            q = q.bind(model_config_json);
        }
        if let Some(diverged_from) = builder.diverged_from {
            q = q.bind(diverged_from);
        }
        if let Some(branch_point_msg_uid) = builder.branch_point_msg_uid {
            q = q.bind(branch_point_msg_uid);
        }
        if let Some(parent_session_id) = builder.parent_session_id {
            q = q.bind(parent_session_id);
        }
        // Same relative position as the clause pair appended above: the tier's
        // one placeholder, then the reason's three — the dominance test, the
        // escalated value, and the value written when the row was assignable.
        if let Some((to, reason)) = builder.privacy_raise {
            q = q.bind(to.as_sql());
            q = q.bind(reason.clone());
            q = q.bind(reason.clone());
            q = q.bind(reason);
        }

        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;
        q = q.bind(&builder.session_id);
        q.execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    async fn get_conversation(&self, session_id: &str) -> Result<Conversation> {
        // BR-7: hydrating is the default, so every existing consumer — the UI
        // transcript, exports, a resumed agent — sees exactly the bytes it saw
        // before externalization existed. `BIOROUTER_SESSION_BLOB_LAZY_LOAD`
        // opts into the lazy read, where an oversized tool result stays a stub
        // and the model pulls it back with `platform__read_session_blob`.
        self.get_conversation_inner(session_id, !message_blobs::lazy_load_enabled())
            .await
    }

    async fn get_conversation_inner(
        &self,
        session_id: &str,
        hydrate_blobs: bool,
    ) -> Result<Conversation> {
        let pool = self.pool().await?;
        let rows = sqlx::query_as::<_, (String, String, i64, Option<String>, Option<String>)>(
            "SELECT role, content_json, created_timestamp, metadata_json, msg_uid FROM messages WHERE session_id = ? ORDER BY id",
        )
            .bind(session_id)
            .fetch_all(pool)
            .await?;

        // The payloads to splice back into any externalized tool result. Fetched
        // once, and only when a row actually carries a stub — a session that
        // never externalized anything (every session before this schema, and
        // every ordinary one after it) pays a substring scan and nothing else.
        let hydrate = hydrate_blobs
            && rows
                .iter()
                .any(|(_, content_json, ..)| message_blobs::content_json_has_stub(content_json));
        let blobs = if hydrate {
            self.load_blobs(session_id).await?
        } else {
            HashMap::new()
        };

        let mut messages = Vec::new();
        for (idx, (role_str, content_json, created_timestamp, metadata_json, msg_uid)) in
            rows.into_iter().enumerate()
        {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => continue,
            };

            let mut content: Vec<MessageContent> = serde_json::from_str(&content_json)?;
            message_blobs::hydrate(&mut content, &blobs);
            let metadata = metadata_json
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();

            let mut message = Message::new(role, created_timestamp, content);
            message.metadata = metadata;
            // Dual-read: prefer the durable `msg_uid`; fall back to the legacy
            // positional id only for a row an in-flight upgrade hasn't
            // backfilled yet (migration 14 backfills all existing rows).
            let id = msg_uid.unwrap_or_else(|| format!("msg_{}_{}", session_id, idx));
            message = message.with_id(id);
            messages.push(message);
        }

        Ok(Conversation::new_unvalidated(messages))
    }

    /// Every externalized payload of one session, keyed by blob handle (BR-7).
    async fn load_blobs(&self, session_id: &str) -> Result<HashMap<String, String>> {
        let pool = self.pool().await?;
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT blob_uid, content FROM message_blobs WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// One externalized payload, by handle. The lazy read path's retrieval seam:
    /// what `platform__read_session_blob` calls when the model asks for the full
    /// output behind a stub.
    async fn get_message_blob(&self, session_id: &str, blob_uid: &str) -> Result<Option<String>> {
        let pool = self.pool().await?;
        let content = sqlx::query_scalar::<_, String>(
            "SELECT content FROM message_blobs WHERE session_id = ? AND blob_uid = ?",
        )
        .bind(session_id)
        .bind(blob_uid)
        .fetch_optional(pool)
        .await?;
        Ok(content)
    }

    /// Returns the **effective** `msg_uid` the message was stored under, so a
    /// caller keeping the message in memory can adopt it (#41). Usually the
    /// caller-supplied id (or a freshly minted one when there was none); only
    /// a uid collision with *different* content re-mints.
    async fn add_message(&self, session_id: &str, message: &Message) -> Result<String> {
        // Runs on the turn path once per message (including every tool
        // response); the transaction covers the message row, blob spill and
        // FTS index write, so it is a plausible per-tool-call fixed cost.
        let _phase = crate::agents::phase_timing::Phase::start("session.add_message");

        // Persist the message's stable id, minting a fresh UUIDv7 when the
        // caller didn't supply one (BR-45).
        let msg_uid = message.id.clone().unwrap_or_else(new_message_id);
        match self.insert_message(session_id, message, &msg_uid).await {
            Ok(()) => Ok(msg_uid),
            Err(err) if is_msg_uid_unique_violation(&err) => {
                // #41: an EXACT replay (same uid, identical role + content +
                // metadata — e.g. a caller retrying a write it believes
                // failed) is idempotent success, not an anomaly. No second
                // row, and the in-memory id already agrees.
                if self
                    .existing_row_matches(session_id, message, &msg_uid)
                    .await?
                {
                    debug!(
                        session_id,
                        %msg_uid,
                        "message uid already persisted with identical content; \
                         treating the replay as success"
                    );
                    return Ok(msg_uid);
                }
                // #41 resilience: a caller-supplied id that already exists in
                // this session with DIFFERENT content (an id-reuse bug
                // upstream — a decoder stamping one shared id on several
                // messages) must degrade to a logged anomaly with a re-minted
                // uid, not abort the whole turn. Retried exactly once with a
                // freshly minted UUIDv7, which cannot collide again. The
                // fresh uid is returned so the caller's in-memory message can
                // adopt it.
                let fresh_uid = new_message_id();
                warn!(
                    session_id,
                    old_uid = %msg_uid,
                    new_uid = %fresh_uid,
                    "message uid already exists in this session; retrying the \
                     insert with a re-minted uid instead of failing the turn"
                );
                self.insert_message(session_id, message, &fresh_uid).await?;
                Ok(fresh_uid)
            }
            Err(err) => Err(err),
        }
    }

    /// Whether the row already stored under `msg_uid` is identical to what
    /// inserting `message` would store (role + created_timestamp + content +
    /// metadata) — i.e. the insert is an exact replay, not an id collision
    /// between two distinct messages.
    ///
    /// `created_timestamp` is part of the comparison (#41): two genuinely
    /// distinct messages that happen to share uid, role, content and metadata
    /// but were created at different times must NOT be collapsed into one —
    /// treating the second as a replay would silently drop it.
    async fn existing_row_matches(
        &self,
        session_id: &str,
        message: &Message,
        msg_uid: &str,
    ) -> Result<bool> {
        let pool = self.pool().await?;
        let row = sqlx::query_as::<_, (String, String, i64, Option<String>)>(
            "SELECT role, content_json, created_timestamp, metadata_json FROM messages \
             WHERE session_id = ? AND msg_uid = ?",
        )
        .bind(session_id)
        .bind(msg_uid)
        .fetch_optional(pool)
        .await?;
        let Some((row_role, row_content, row_created, row_metadata)) = row else {
            return Ok(false);
        };

        if row_role != role_to_string(&message.role) || row_created != message.created {
            return Ok(false);
        }

        // `metadata_json` is nullable for rows migrated from older schemas
        // (#41): a NULL there is the stored form of "no metadata was
        // recorded", which can only be an exact replay of a message whose
        // in-memory metadata is still the default. Decoding it as a bare
        // `String` made the replay probe *error* on such rows, aborting the
        // very turn the idempotent-replay path exists to save.
        let metadata_matches = match row_metadata {
            Some(row_metadata) => row_metadata == serde_json::to_string(&message.metadata)?,
            None => message.metadata == crate::conversation::message::MessageMetadata::default(),
        };
        if !metadata_matches {
            return Ok(false);
        }

        self.stored_content_matches(session_id, &row_content, message)
            .await
    }

    /// Whether `row_content` (the stored `content_json`) and the candidate
    /// message's content are the same payload.
    ///
    /// The comparison must be *stable* across externalization (#41):
    /// [`message_blobs::externalize`] mints a fresh blob uid per call, so
    /// serializing a freshly-externalized candidate could never equal the
    /// stored row for an oversized message — every large-message replay
    /// compared unequal and was re-inserted under a re-minted uid. Instead,
    /// compare the pre-externalization forms: hydrate the stored stubs back
    /// to their payloads (and any stubs the candidate itself carries, e.g. a
    /// re-persisted already-externalized conversation) and compare those.
    async fn stored_content_matches(
        &self,
        session_id: &str,
        row_content: &str,
        message: &Message,
    ) -> Result<bool> {
        if !message_blobs::content_json_has_stub(row_content) {
            return Ok(row_content == serde_json::to_string(&message.content)?);
        }

        let mut stored: Vec<MessageContent> = serde_json::from_str(row_content)?;
        let mut candidate = message.content.clone();
        let mut uids = message_blobs::referenced_uids(&stored);
        uids.extend(message_blobs::referenced_uids(&candidate));
        let mut blobs = std::collections::HashMap::new();
        for uid in uids {
            if let Some(content) = self.get_message_blob(session_id, &uid).await? {
                blobs.insert(uid, content);
            }
        }
        message_blobs::hydrate(&mut stored, &blobs);
        message_blobs::hydrate(&mut candidate, &blobs);
        Ok(serde_json::to_string(&stored)? == serde_json::to_string(&candidate)?)
    }

    /// One attempt at the transactional message insert (row + blob spill +
    /// FTS index + session touch), with an explicit `msg_uid`. Split out of
    /// [`Self::add_message`] so a uid collision can be retried with a fresh
    /// uid on a clean transaction.
    async fn insert_message(
        &self,
        session_id: &str,
        message: &Message,
        msg_uid: &str,
    ) -> Result<()> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;

        let metadata_json = serde_json::to_string(&message.metadata)?;

        // BR-7: lift an oversized tool-result payload into the blob side table
        // so the `messages` row stays small. `None` (the common case) stores the
        // content exactly as before, with no extra allocation.
        let externalized = message_blobs::externalize(&message.content);
        let (content_json, blobs) = match &externalized {
            Some((content, blobs)) => (serde_json::to_string(content)?, blobs.as_slice()),
            None => (serde_json::to_string(&message.content)?, [].as_slice()),
        };

        let insert = sqlx::query(
            r#"
            INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json, msg_uid)
            VALUES (?, ?, ?, ?, ?, ?)
        "#,
        )
        .bind(session_id)
        .bind(role_to_string(&message.role))
        .bind(content_json)
        .bind(message.created)
        .bind(metadata_json)
        .bind(msg_uid)
        .execute(&mut *tx)
        .await?;

        // Same transaction as the message row: a stub can never be persisted
        // without the payload it points at.
        Self::insert_blobs(&mut tx, session_id, blobs).await?;

        // Keep the FTS recall index in sync with the new row (BR-17). Indexed
        // from the *original* message: recall renders a tool response as a
        // placeholder, so externalization cannot change what is searchable.
        let fts_available = Self::messages_fts_exists(&mut *tx).await;
        Self::index_message_fts(
            &mut tx,
            session_id,
            insert.last_insert_rowid(),
            message,
            fts_available,
        )
        .await?;

        sqlx::query("UPDATE sessions SET updated_at = datetime('now') WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    /// True when the FTS5 mirror table exists (created by schema migration 15).
    /// The read path guards on this too; a DB that reached its version without
    /// `messages_fts` (e.g. a future migration renumber, or a partial upgrade)
    /// must degrade gracefully instead of hard-failing every message save.
    async fn messages_fts_exists<'e, E>(executor: E) -> bool
    where
        E: sqlx::Executor<'e, Database = Sqlite>,
    {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
        )
        .fetch_one(executor)
        .await
        .map(|c| c > 0)
        .unwrap_or(false)
    }

    /// Insert one message's flattened text into the FTS recall index, within
    /// the caller's transaction. Only user-visible, non-empty messages are
    /// indexed (BR-17). `fts_available` is resolved once by the caller so a
    /// bulk rewrite doesn't re-probe the catalog per message.
    async fn index_message_fts(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        session_id: &str,
        message_id: i64,
        message: &Message,
        fts_available: bool,
    ) -> Result<()> {
        if !fts_available || !message.metadata.user_visible {
            return Ok(());
        }
        let text = chat_fts::extract_searchable_text(&message.content);
        if text.is_empty() {
            return Ok(());
        }
        sqlx::query(MESSAGES_FTS_INSERT)
            .bind(&text)
            .bind(session_id)
            .bind(message_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn replace_conversation_inner(
        pool: &Pool<Sqlite>,
        session_id: &str,
        conversation: &Conversation,
        guard: RewriteGuard<'_>,
    ) -> Result<(ReplaceOutcome, Vec<Message>)> {
        let mut tx = pool.begin().await?;

        // LOAD-BEARING FIRST STATEMENT, AND IT MUST BE A WRITE. DO NOT REORDER.
        //
        // sqlx's `pool.begin()` emits a bare (DEFERRED) `BEGIN`. Under WAL, a
        // deferred transaction that READS first pins a read snapshot; if another
        // connection commits before our first write, the upgrade to a writer
        // returns SQLITE_BUSY_SNAPSHOT *immediately* — measured at 0.0000s,
        // i.e. the 5s `busy_timeout` is bypassed, because a busy handler is not
        // consulted for a snapshot upgrade. Opening with a WRITE takes the
        // single per-file write lock up front, so any SELECT that follows reads
        // true latest-committed state and the DELETE below cannot fail that way.
        // (A concurrent writer then blocks on the busy timeout, which is
        // correct.) The freshness guard added on top of this relies on it.
        //
        // It also fixes a real gap: this rewrite never bumped
        // `sessions.updated_at`, so a compaction or an edit was invisible to the
        // `ORDER BY updated_at DESC` session list even though it changed the
        // session's content. `insert_message` has always bumped it.
        let touched = sqlx::query("UPDATE sessions SET updated_at = datetime('now') WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // The freshness guard, evaluated UNDER the write lock the statement
        // above just took — so nothing can interleave between the check and the
        // DELETE. `Unconditional` skips it entirely and keeps this function's
        // original semantics byte for byte.
        let mut outcome = ReplaceOutcome::Replaced;
        let mut recovered: Vec<Message> = Vec::new();
        if let RewriteGuard::PreserveTail { basis, known } = guard {
            if touched.rows_affected() == 0 {
                // Decided before any destructive work has happened.
                tx.rollback().await?;
                return Ok((ReplaceOutcome::SessionNotFound, Vec::new()));
            }

            // Row identity FIRST (#51 W3). A session id outlives the session:
            // `/reset` History empties `sessions`, and the next
            // `create_session` on the same day hands the id straight back. A
            // rewrite that snapshotted the previous occupant must never be
            // allowed to reason about rowids in the new one's message set —
            // `(count, max_rowid)` is only meaningful within one incarnation.
            let incarnation = Self::read_incarnation(&mut tx, session_id).await?;
            if incarnation != basis.incarnation {
                tx.rollback().await?;
                return Ok((ReplaceOutcome::Stale, Vec::new()));
            }

            // Prefix integrity: every message the basis covered must still be
            // there, unmoved. A concurrent truncate lowers this; a concurrent
            // wholesale rewrite renumbers every row and drives it to 0. Either
            // way there is no sound prefix to merge onto, so refuse.
            let prefix = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE session_id = ? AND id <= ?",
            )
            .bind(session_id)
            .bind(basis.max_rowid)
            .fetch_one(&mut *tx)
            .await?;
            if prefix != basis.count {
                tx.rollback().await?;
                return Ok((ReplaceOutcome::Stale, Vec::new()));
            }

            recovered = Self::scan_foreign_tail(&mut tx, session_id, basis, known).await?;
            if !recovered.is_empty() {
                outcome = ReplaceOutcome::ReplacedPreservingTail {
                    preserved: recovered.len(),
                };
            }
        }

        // ONE merged list, built BEFORE the insert loop. Appending the recovered
        // messages in a second pass instead would run the blob accounting below
        // without their handles, so `sweep_orphan_blobs` would delete the
        // payload of a recovered tool response and leave a dangling stub —
        // silent, and only visible on the next read.
        let mut merged: Vec<Message> = conversation
            .messages()
            .iter()
            .cloned()
            .chain(recovered)
            .collect();

        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // Rebuild the FTS recall index for this session in lockstep with the
        // message rewrite, so a compacted/edited session stays searchable
        // without double-counting (BR-17). Skip entirely when the mirror table
        // is absent so message rewrites still succeed on such a DB.
        let fts_available = Self::messages_fts_exists(&mut *tx).await;
        if fts_available {
            sqlx::query("DELETE FROM messages_fts WHERE session_id = ?")
                .bind(session_id)
                .execute(&mut *tx)
                .await?;
        }

        // Every blob still referenced after the rewrite (BR-7): the handles that
        // survive inside kept stubs, plus the ones minted below. Anything else
        // belonged to a message this rewrite dropped and is swept at the end.
        let mut live_blob_uids: Vec<String> = Vec::new();

        for message in merged.iter_mut() {
            let metadata_json = serde_json::to_string(&message.metadata)?;
            // PRESERVE each kept message's stable id across the rewrite (this is
            // the exact op — DELETE + re-INSERT — that used to renumber ids).
            // Only a newly-minted message (e.g. a compaction summary) with no id
            // gets a fresh one (BR-45).
            let msg_uid = message.id.clone().unwrap_or_else(new_message_id);

            let externalized = message_blobs::externalize(&message.content);
            let (content_json, blobs) = match &externalized {
                Some((content, blobs)) => (serde_json::to_string(content)?, blobs.as_slice()),
                None => (serde_json::to_string(&message.content)?, [].as_slice()),
            };
            live_blob_uids.extend(message_blobs::referenced_uids(
                externalized
                    .as_ref()
                    .map_or(message.content.as_slice(), |(content, _)| {
                        content.as_slice()
                    }),
            ));

            let insert = sqlx::query(
                r#"
            INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json, msg_uid)
            VALUES (?, ?, ?, ?, ?, ?)
        "#,
            )
            .bind(session_id)
            .bind(role_to_string(&message.role))
            .bind(content_json)
            .bind(message.created)
            .bind(metadata_json)
            .bind(msg_uid.clone())
            .execute(&mut *tx)
            .await?;

            Self::insert_blobs(&mut tx, session_id, blobs).await?;

            Self::index_message_fts(
                &mut tx,
                session_id,
                insert.last_insert_rowid(),
                message,
                fts_available,
            )
            .await?;

            // Stamp the effective uid so the conversation handed back to the
            // caller carries the same ids as the rows (#41's contract, applied
            // to the rewrite path).
            message.id = Some(msg_uid);
        }

        // A conversation written here can carry stubs minted under *another*
        // session — a diverge/copy re-inserts the parent's messages verbatim
        // into the child. Give the child its own row for each such payload
        // before the sweep, so the two sessions' lifetimes stay independent.
        Self::adopt_blobs(&mut tx, session_id, &live_blob_uids).await?;
        Self::sweep_orphan_blobs(&mut tx, session_id, &live_blob_uids).await?;

        tx.commit().await?;
        Ok((outcome, merged))
    }

    /// The messages stored since `basis` that the caller's view never contained
    /// — i.e. what another writer appended while the caller was computing its
    /// rewrite. Read inside the rewrite's own transaction, under the write lock.
    ///
    /// Decoding mirrors `get_conversation_inner` exactly, INCLUDING the legacy
    /// `msg_{session}_{idx}` fallback for a row an in-flight upgrade has not
    /// backfilled — an id that decoded differently here would never match
    /// `known` and would look foreign forever. The absolute index is
    /// `basis.count + relative`, which is exact because the prefix check the
    /// caller just ran proved there are exactly `basis.count` rows at or below
    /// the watermark, and both reads order by `id`.
    ///
    /// Blobs are deliberately NOT hydrated. A recovered row's `content_json`
    /// may carry a BR-7 stub; re-inserting the stub verbatim means
    /// `externalize` mints nothing, the existing handle joins `live_blob_uids`,
    /// and the sweep spares it. Hydrating and re-externalizing would mint a
    /// duplicate blob row for the same payload.
    ///
    /// That makes this function's output STORAGE-shaped, not caller-shaped.
    /// The conversation handed back to a live turn is hydrated separately, on
    /// the returned copy only — see
    /// [`SessionStorage::hydrate_recovered_tail`]. Do not "simplify" by
    /// hydrating here.
    ///
    /// Parsing `content_json` into `Vec<MessageContent>` only for the insert
    /// loop to re-serialize it looks like a removable round-trip. It is not:
    /// the parsed form is what `referenced_uids` reads for blob liveness, what
    /// `index_message_fts` extracts search text from, and what the returned
    /// `Conversation` is made of. Only the *re-serialization* is avoidable, and
    /// only by threading a parallel raw-JSON array through the insert loop and
    /// branching its `content_json` binding — which costs more clarity than the
    /// microseconds it saves on a rare path, and gives up the guarantee that
    /// every row this rewrite writes went through one serializer.
    async fn scan_foreign_tail(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        session_id: &str,
        basis: ConversationRevision,
        known: &Conversation,
    ) -> Result<Vec<Message>> {
        let rows = sqlx::query_as::<_, (String, String, i64, Option<String>, Option<String>)>(
            "SELECT role, content_json, created_timestamp, metadata_json, msg_uid \
             FROM messages WHERE session_id = ? AND id > ? ORDER BY id",
        )
        .bind(session_id)
        .bind(basis.max_rowid)
        .fetch_all(&mut **tx)
        .await?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let known_uids: std::collections::HashSet<&str> = known
            .messages()
            .iter()
            .filter_map(|m| m.id.as_deref())
            .collect();

        let mut foreign = Vec::new();
        for (relative, (role_str, content_json, created_timestamp, metadata_json, msg_uid)) in
            rows.into_iter().enumerate()
        {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                // Same as the read path: a row with an unrecognized role is not
                // representable and is dropped by any rewrite.
                _ => continue,
            };
            let id = msg_uid.unwrap_or_else(|| {
                format!("msg_{}_{}", session_id, basis.count as usize + relative)
            });
            if known_uids.contains(id.as_str()) {
                continue;
            }
            let content: Vec<MessageContent> = serde_json::from_str(&content_json)?;
            let metadata = metadata_json
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();
            let mut message = Message::new(role, created_timestamp, content);
            message.metadata = metadata;
            foreign.push(message.with_id(id));
        }
        Ok(foreign)
    }

    /// Write the payloads lifted out of one message, in the caller's transaction.
    async fn insert_blobs(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        session_id: &str,
        blobs: &[message_blobs::PendingBlob],
    ) -> Result<()> {
        for blob in blobs {
            sqlx::query(
                r#"
                INSERT OR REPLACE INTO message_blobs (blob_uid, session_id, created_at, bytes, content)
                VALUES (?, ?, ?, ?, ?)
            "#,
            )
            .bind(&blob.uid)
            .bind(session_id)
            .bind(Utc::now().timestamp())
            .bind(blob.bytes())
            .bind(&blob.content)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Copy any referenced blob this session does not own yet from whichever
    /// session does (the parent of a diverge/copy). No-op for the common case
    /// where every handle was minted here.
    async fn adopt_blobs(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        session_id: &str,
        uids: &[String],
    ) -> Result<()> {
        for uid in uids {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO message_blobs (blob_uid, session_id, created_at, bytes, content)
                SELECT blob_uid, ?, created_at, bytes, content
                FROM message_blobs WHERE blob_uid = ? LIMIT 1
            "#,
            )
            .bind(session_id)
            .bind(uid)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Drop this session's blobs that no surviving message points at — the
    /// payloads of tool responses that compaction (or an edit) just removed.
    /// Without this the side table would only ever grow, trading one kind of DB
    /// bloat for another.
    async fn sweep_orphan_blobs(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        session_id: &str,
        live_uids: &[String],
    ) -> Result<()> {
        if live_uids.is_empty() {
            sqlx::query("DELETE FROM message_blobs WHERE session_id = ?")
                .bind(session_id)
                .execute(&mut **tx)
                .await?;
            return Ok(());
        }

        let placeholders = vec!["?"; live_uids.len()].join(", ");
        let sql = format!(
            "DELETE FROM message_blobs WHERE session_id = ? AND blob_uid NOT IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql).bind(session_id);
        for uid in live_uids {
            query = query.bind(uid);
        }
        query.execute(&mut **tx).await?;
        Ok(())
    }

    pub async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()> {
        let pool = self.pool().await?;
        Self::replace_conversation_inner(
            pool,
            session_id,
            conversation,
            RewriteGuard::Unconditional,
        )
        .await
        .map(|_| ())
    }

    /// Whole-history rewrite that carries over anything appended since `basis`.
    /// See [`SessionManager::replace_conversation_preserving_tail`].
    ///
    /// PRECONDITION: `basis` and `known` must come from ONE
    /// [`Self::snapshot_for_rewrite`] of `session_id`. In particular a
    /// default/synthetic `(0, 0)` basis paired with a NON-EMPTY session is
    /// incoherent and is not refused: the prefix check trivially passes
    /// (`COUNT(*) WHERE id <= 0` is 0), every existing row scans as foreign,
    /// and the whole real history is appended AFTER `replacement` — i.e. the
    /// replacement is silently prepended to the transcript rather than
    /// replacing it. No in-tree caller constructs a basis by hand (all three
    /// compaction sites thread one through from `snapshot_for_rewrite`); keep
    /// it that way.
    pub async fn replace_conversation_preserving_tail(
        &self,
        session_id: &str,
        replacement: &Conversation,
        basis: ConversationRevision,
        known: &Conversation,
    ) -> Result<(ReplaceOutcome, Conversation)> {
        let pool = self.pool().await?;
        let (outcome, mut stored) = Self::replace_conversation_inner(
            pool,
            session_id,
            replacement,
            RewriteGuard::PreserveTail { basis, known },
        )
        .await?;
        if !outcome.stored() {
            return Ok((outcome, replacement.clone()));
        }
        if let ReplaceOutcome::ReplacedPreservingTail { preserved } = outcome {
            self.hydrate_recovered_tail(session_id, &mut stored, preserved)
                .await?;
        }
        Ok((outcome, Conversation::new_unvalidated(stored)))
    }

    /// Splice the payloads back into the tail `scan_foreign_tail` recovered,
    /// on the copy handed BACK to the caller only.
    ///
    /// The rewrite deliberately re-inserts a recovered row's `content_json`
    /// verbatim, stubs and all, so `externalize` mints no duplicate blob (see
    /// `scan_foreign_tail`). But those same `Message` objects are what the
    /// caller adopts as live turn state — `conversation = stored` and
    /// `AgentEvent::HistoryReplaced(stored)` in the agent loop. Handing back the
    /// stub means the rest of the turn reasons over a ~1 KB placeholder where a
    /// concurrently-appended oversized tool response should be, and the UI
    /// transcript renders the placeholder too, until a reload heals it.
    ///
    /// So: hydrate the returned copy, never the one written. Mirrors
    /// `get_conversation_inner` exactly, including honouring
    /// `BIOROUTER_SESSION_BLOB_LAZY_LOAD` — under lazy load a stub is what a
    /// re-read would give, and the model pulls the payload back with
    /// `platform__read_session_blob`.
    async fn hydrate_recovered_tail(
        &self,
        session_id: &str,
        merged: &mut [Message],
        preserved: usize,
    ) -> Result<()> {
        if preserved == 0 || message_blobs::lazy_load_enabled() {
            return Ok(());
        }
        // The recovered rows are exactly the suffix: `merged` is
        // `replacement ++ recovered`, built in that order by the rewrite.
        let start = merged.len().saturating_sub(preserved);
        let tail = &mut merged[start..];
        // A session that never externalized anything pays one uid scan and no
        // query at all — the same "only when a row actually carries a stub"
        // discipline as the read path.
        if !tail
            .iter()
            .any(|m| !message_blobs::referenced_uids(&m.content).is_empty())
        {
            return Ok(());
        }
        let blobs = self.load_blobs(session_id).await?;
        for message in tail.iter_mut() {
            message_blobs::hydrate(&mut message.content, &blobs);
        }
        Ok(())
    }

    /// [`ConversationRevision`] of one session, read from the pool.
    pub async fn conversation_revision(&self, session_id: &str) -> Result<ConversationRevision> {
        let pool = self.pool().await?;
        Self::read_revision(pool, session_id).await
    }

    /// The revision read itself, over any executor — the pool for the public
    /// reader, the open transaction for the guard inside a rewrite.
    async fn read_revision<'e, E>(executor: E, session_id: &str) -> Result<ConversationRevision>
    where
        E: sqlx::Executor<'e, Database = Sqlite>,
    {
        // The incarnation rides along as an uncorrelated scalar subquery — one
        // primary-key probe of `sessions`, evaluated once, so this is still a
        // single round trip and a single scan of `idx_messages_session`.
        let (count, max_rowid, incarnation) = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT COUNT(*), IFNULL(MAX(id), 0), \
             IFNULL((SELECT incarnation FROM sessions WHERE id = ?), 0) \
             FROM messages WHERE session_id = ?",
        )
        .bind(session_id)
        .bind(session_id)
        .fetch_one(executor)
        .await?;
        Ok(ConversationRevision {
            incarnation,
            count,
            max_rowid,
        })
    }

    /// The session row's incarnation token, read over the rewrite's own
    /// transaction. `0` for a row that predates the column (or does not exist).
    async fn read_incarnation(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        session_id: &str,
    ) -> Result<i64> {
        let incarnation = sqlx::query_scalar::<_, i64>(
            "SELECT IFNULL(incarnation, 0) FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&mut **tx)
        .await?
        .unwrap_or(0);
        Ok(incarnation)
    }

    async fn list_sessions_by_types(&self, types: &[SessionType]) -> Result<Vec<Session>> {
        self.list_sessions_by_types_maybe_empty(types, false).await
    }

    /// `list_sessions_by_types`, with the `messages` join selectable.
    ///
    /// ⚠ **`include_empty` exists because a subagent that produced nothing was
    /// invisible.** The historical query INNER JOINs `messages`, so a row with no
    /// message is not returned at all — and that is right for History (an
    /// "Untitled chat" placeholder must not appear in the sidebar) but wrong for
    /// `biorouter session list --subagents`, which is the **only** surface that
    /// can show a subagent run at all. A child that was spawned and died before
    /// its first message simply vanished there, so the one place a user could
    /// have gone looking for it reported that it never existed.
    ///
    /// `COUNT(m.id)` ignores NULLs, so the LEFT JOIN still yields `message_count
    /// = 0` rather than 1 — the same reasoning `list_session_summaries`'
    /// `include_empty` already records.
    ///
    /// The historical entry point above delegates with `false`, byte for byte,
    /// so the sidebar route (`biorouter-server`'s `/sessions`) and
    /// `commands/term.rs` are unchanged and no caller outside this file had to
    /// move.
    async fn list_sessions_by_types_maybe_empty(
        &self,
        types: &[SessionType],
        include_empty: bool,
    ) -> Result<Vec<Session>> {
        if types.is_empty() {
            return Ok(Vec::new());
        }

        let join = if include_empty {
            "LEFT JOIN messages m ON s.id = m.session_id"
        } else {
            "INNER JOIN messages m ON s.id = m.session_id"
        };
        let placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query = format!(
            r#"
            SELECT s.id, s.working_dir, s.name, s.description, s.user_set_name, s.session_type, s.created_at, s.updated_at, s.extension_data,
                   s.total_tokens, s.input_tokens, s.output_tokens,
                   s.accumulated_total_tokens, s.accumulated_input_tokens, s.accumulated_output_tokens,
                   s.schedule_id, s.workflow_json, s.user_workflow_values_json,
                   s.provider_name, s.model_config_json, s.diverged_from, s.parent_session_id,
                   s.privacy_tier, s.privacy_reason,
                   COUNT(m.id) as message_count
            FROM sessions s
            {join}
            WHERE s.session_type IN ({placeholders})
            GROUP BY s.id
            ORDER BY s.updated_at DESC
            "#
        );

        let mut q = sqlx::query_as::<_, Session>(&query);
        for t in types {
            q = q.bind(t.to_string());
        }

        let pool = self.pool().await?;
        q.fetch_all(pool).await.map_err(Into::into)
    }

    async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.list_sessions_by_types(&[SessionType::User, SessionType::Scheduled])
            .await
    }

    async fn list_session_summaries(
        &self,
        limit: u32,
        offset: u32,
        include_subagents: bool,
        include_empty: bool,
    ) -> Result<Vec<SessionSummary>> {
        let type_filter = if include_subagents {
            "('user', 'scheduled', 'sub_agent')"
        } else {
            "('user', 'scheduled')"
        };
        // The sidebar deliberately hides message-less sessions (an INNER JOIN on
        // `messages`) so "Untitled chat" placeholders never appear in History.
        // `workspace_list` needs the opposite: a session `workspace_open` just
        // created has no message yet and must still be listable. `COUNT(m.id)`
        // ignores NULLs, so the LEFT JOIN still yields 0.
        let join = if include_empty {
            "LEFT JOIN messages m ON s.id = m.session_id"
        } else {
            "INNER JOIN messages m ON s.id = m.session_id"
        };
        let query = format!(
            r#"
            SELECT s.id,
                   s.working_dir,
                   COALESCE(NULLIF(s.name, ''), NULLIF(s.description, ''), 'Untitled chat') AS name,
                   s.user_set_name,
                   s.created_at,
                   s.updated_at,
                   s.parent_session_id,
                   s.session_type,
                   s.diverged_from,
                   s.privacy_tier,
                   COUNT(m.id) AS message_count
            FROM sessions s
            {join}
            WHERE s.session_type IN {type_filter}
            GROUP BY s.id
            ORDER BY s.updated_at DESC, s.id ASC
            LIMIT ? OFFSET ?
            "#
        );

        let pool = self.pool().await?;
        sqlx::query_as::<_, SessionSummary>(&query)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(pool)
            .await
            .map_err(Into::into)
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;

        // Write FIRST, then decide whether the session existed. `pool.begin()`
        // issues a DEFERRED transaction, so the statement that opens it decides
        // what lock it takes: a leading `SELECT` pins a WAL read snapshot, and
        // the later `DELETE` then has to *upgrade* to the write lock. If any
        // other connection committed in between, SQLite refuses that upgrade
        // with SQLITE_BUSY_SNAPSHOT — and the busy handler is NOT consulted for
        // it, so the pool's five-second `busy_timeout` cannot save it. That
        // surfaced as `(code: 5) database is locked` on roughly one in three
        // concurrent test runs, and would surface the same way in the daemon
        // whenever a delete raced a message write. Opening with the `DELETE`
        // takes the write lock up front, where the busy handler does apply.
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // The session's externalized tool-result payloads go with it (BR-7).
        sqlx::query("DELETE FROM message_blobs WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // ...and so does its usage ledger (F10). Each row names the model and
        // provider that answered one turn, and when — a record of a chat the
        // user chose to delete — and leaving them cost twice more: Home kept
        // counting them, and because `create_session` mints `<day>_<MAX(N)+1>`,
        // deleting the newest chat of the day hands its id to the next one,
        // which inherited the whole ledger. A `JOIN sessions` at read time
        // cannot tell those rows apart; deleting them here can.
        //
        // The tokens themselves were spent, and a delete does not refund them
        // at the provider — the whole reason the Usage panel exists is to hold
        // Biorouter's numbers against that meter (issue #1). So the billable
        // turns are first folded into `deleted_chat_usage`, which keeps sums
        // per local day, model and provider and nothing that identifies the
        // chat. See [`BILLABLE_USAGE`] for which totals read it.
        sqlx::query(&Self::fold_into_deleted_chat_usage_sql(
            "te.session_id = ?1",
        ))
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM token_events WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // ...and every other row that exists only for this chat, for the same
        // reason: its text in the recall index, its checkpoints, and the
        // cross-institution flows the user accepted in it.
        Self::delete_chat_side_rows(&mut tx, session_id).await?;

        let removed = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        if removed.rows_affected() == 0 {
            // Dropping `tx` rolls back, so the deletes above are undone and an
            // unknown id still writes nothing — same contract as before.
            return Err(anyhow::anyhow!("Session not found"));
        }

        tx.commit().await?;
        self.remove_checkpoint_repository(session_id).await;
        Ok(())
    }

    /// Delete every row of one chat outside `messages`, `message_blobs` and
    /// `token_events`, inside the caller's transaction.
    ///
    /// Each table is keyed by the chat's id, and a row that outlived the chat
    /// was not merely retained: `create_session` minted `<day>_<MAX(N)+1>`, so
    /// the next chat to be handed the id read the row as its own.
    ///
    /// - `messages_fts` — the recall index's copy of the chat's user-visible
    ///   text, word for word (BR-17). `replace_conversation` and
    ///   `truncate_conversation` already kept it in step; a delete left all of
    ///   it on disk. Recall joins `messages`, whose rowids are never minted
    ///   twice, so no search could reach it — but the text of a chat the user
    ///   deleted, possibly PHI, stayed in `sessions.db`.
    /// - `checkpoints` — BR-43's records. A new chat under the id listed them,
    ///   and restoring one would have rolled that chat back to a point in
    ///   another. The shadow repository they point into is a directory, and
    ///   goes after the commit: [`Self::remove_checkpoint_repository`].
    /// - `cross_affiliation_grants` — issue #56 DR-26. The one table here that
    ///   is an authorisation rather than a record: a new chat under the id was
    ///   let through Gate C for flows accepted in a chat the user deleted.
    ///   `privacy::grant::GRANT_IS_THE_CHATS_OWN` also refuses such a grant at
    ///   read time, for the ones this delete never sees.
    ///
    /// Not here, deliberately: `classification_audit` survives deletion by
    /// design (privacy-tiers §12.5) — it records what has ever been
    /// declassified on this machine — so it is the reader,
    /// [`Self::NOT_DECLASSIFIED_BY_USER`], that has to tell one chat under an id
    /// from another. A deleted chat's subagent runs are sessions of their own
    /// and are kept, for the reasons
    /// `deleted_chat_side_rows_tests::a_deleted_chats_subagent_runs_are_kept_and_never_adopted`
    /// gives.
    async fn delete_chat_side_rows(
        connection: &mut sqlx::SqliteConnection,
        session_id: &str,
    ) -> Result<()> {
        if Self::messages_fts_exists(&mut *connection).await {
            sqlx::query("DELETE FROM messages_fts WHERE session_id = ?")
                .bind(session_id)
                .execute(&mut *connection)
                .await?;
        }
        for table in ["checkpoints", "cross_affiliation_grants"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE session_id = ?"))
                .bind(session_id)
                .execute(&mut *connection)
                .await?;
        }
        Ok(())
    }

    /// Remove a deleted chat's BR-43 shadow repository — snapshots of its
    /// working tree, file contents and all — from
    /// [`crate::checkpoint::repository_dir`] beside the session database.
    /// `CheckpointManager::gc` was written to do this "on session delete" and
    /// nothing ever called it, so every repository outlived its chat and the
    /// next chat to be handed the id opened it and committed on top.
    ///
    /// After the commit, and best-effort: the rows are already gone, so a
    /// failure leaves unreferenced files rather than a half-deleted chat, and is
    /// logged rather than returned as a failed delete the user would retry
    /// against a chat that no longer exists.
    async fn remove_checkpoint_repository(&self, session_id: &str) {
        let Some(dir) = self
            .session_dir
            .parent()
            .and_then(|data_dir| crate::checkpoint::repository_dir(data_dir, session_id))
        else {
            return;
        };
        let removal = tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err((dir, error)),
            _ => Ok(()),
        })
        .await;
        match removal {
            Ok(Ok(())) => {}
            Ok(Err((dir, error))) => warn!(
                %error,
                dir = %dir.display(),
                "could not remove a deleted chat's checkpoint repository"
            ),
            Err(error) => warn!(%error, "the checkpoint repository removal task failed"),
        }
    }

    async fn count_all_sessions(&self) -> Result<u64> {
        let pool = self.pool().await?;
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
            .fetch_one(pool)
            .await?;
        Ok(count as u64)
    }

    async fn clear_all_sessions(&self) -> Result<u64> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
            .fetch_one(&mut *tx)
            .await?;

        if Self::messages_fts_exists(&mut *tx).await {
            sqlx::query("DELETE FROM messages_fts")
                .execute(&mut *tx)
                .await?;
        }
        // `cross_affiliation_grants` belong to chats like every other table here
        // (issue #56 DR-26); `classification_audit` does not — it survives
        // deletion by design (privacy-tiers §12.5).
        for table in [
            "message_blobs",
            "checkpoints",
            "messages",
            "token_events",
            "deleted_chat_usage",
            "cross_affiliation_grants",
            "sessions",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await?;
        }
        // The AUTOINCREMENT high-water marks are deliberately LEFT ALONE
        // (#51 W3). `messages.id` is what [`ConversationRevision`] is built
        // from, and its whole value is that a rowid is never minted twice for
        // the lifetime of the database. Rewinding `sqlite_sequence` here made a
        // wipe hand the next session the rowids of the one it deleted, so a
        // detached rewrite holding a pre-wipe revision could pass the freshness
        // guard against a brand-new conversation and destroy it. `token_events`
        // rides along for the same reason: its ids are referenced by the usage
        // ledger's own dedupe, and there is nothing to gain from replaying
        // them. A cleared database is empty either way; only the two counters
        // survive, at 8 bytes each.
        tx.commit().await?;
        Ok(count as u64)
    }

    async fn get_insights(&self) -> Result<SessionInsights> {
        let pool = self.pool().await?;

        // Sessions: totals plus 7d/30d windows.
        //
        // The session windows key on `updated_at` deliberately — an active
        // session counts as recent even if it was started earlier. Only
        // user-facing session types are counted, so these tiles agree with the
        // session list rendered beneath them.
        let sessions = sqlx::query_as::<_, (i64, i64, i64)>(
            r#"
            SELECT
              COUNT(*) AS total_sessions,
              COALESCE(SUM(CASE WHEN updated_at >= datetime('now', '-7 days') THEN 1 ELSE 0 END), 0) AS sessions_7d,
              COALESCE(SUM(CASE WHEN updated_at >= datetime('now', '-30 days') THEN 1 ELSE 0 END), 0) AS sessions_30d
            FROM sessions
            WHERE session_type IN ('user', 'scheduled')
            "#,
        )
        .fetch_one(pool)
        .await?;

        // Tokens: summed from per-turn events inside the window.
        //
        // The old query summed each session's WHOLE lifetime total if the session
        // had merely been touched in the window, so a 60-day-old session holding
        // 2,000,000 tokens that received one reply today contributed all
        // 2,000,000 to "past 7 days".
        //
        // Joined to `sessions`: Home counts the activity of chats that still
        // exist, so a deleted chat's tokens leave these tiles with it (F10). The
        // spend stays where it is held against the provider's meter — the Usage
        // panel, which reads [`BILLABLE_USAGE`] instead.
        let tokens = sqlx::query_as::<
            _,
            (
                i64,
                i64,
                Option<i64>,
                i64,
                i64,
                Option<i64>,
                i64,
                i64,
                Option<i64>,
            ),
        >(
            r#"
            SELECT
              COUNT(*),
              COUNT(te.billed_total_tokens),
              SUM(te.billed_total_tokens),
              COUNT(CASE WHEN te.ts >= CAST(strftime('%s', 'now', '-7 days') AS INTEGER) THEN 1 END),
              COUNT(CASE WHEN te.ts >= CAST(strftime('%s', 'now', '-7 days') AS INTEGER) THEN te.billed_total_tokens END),
              SUM(CASE WHEN te.ts >= CAST(strftime('%s', 'now', '-7 days') AS INTEGER) THEN te.billed_total_tokens END),
              COUNT(CASE WHEN te.ts >= CAST(strftime('%s', 'now', '-30 days') AS INTEGER) THEN 1 END),
              COUNT(CASE WHEN te.ts >= CAST(strftime('%s', 'now', '-30 days') AS INTEGER) THEN te.billed_total_tokens END),
              SUM(CASE WHEN te.ts >= CAST(strftime('%s', 'now', '-30 days') AS INTEGER) THEN te.billed_total_tokens END)
            FROM token_events te
            JOIN sessions s ON s.id = te.session_id
            WHERE te.session_type IN ('user', 'scheduled')
            "#,
        )
        .fetch_one(pool)
        .await?;

        Ok(SessionInsights {
            total_sessions: sessions.0 as usize,
            total_tokens: complete_sum(tokens.0, tokens.1, tokens.2),
            sessions_last_7_days: sessions.1.max(0) as usize,
            sessions_last_30_days: sessions.2.max(0) as usize,
            tokens_last_7_days: complete_sum(tokens.3, tokens.4, tokens.5),
            tokens_last_30_days: complete_sum(tokens.6, tokens.7, tokens.8),
        })
    }

    /// Record one turn's usage. Append-only: never updated, and deleted only
    /// with its chat — `delete_session` first folds it into the anonymous
    /// `deleted_chat_usage` — or by `clear_all_sessions`.
    #[allow(clippy::too_many_arguments)]
    async fn record_token_event(
        &self,
        session_id: &str,
        input: Option<i32>,
        output: Option<i32>,
        total: i64,
        model: Option<&str>,
        provider: Option<&str>,
        cache_read: Option<i32>,
        cache_creation: Option<i32>,
    ) -> Result<()> {
        let pool = self.pool().await?;
        // An empty model/provider string is stored as NULL so it aggregates with
        // the genuinely-unknown rows rather than as a distinct "" group.
        let model = model.filter(|m| !m.is_empty());
        let provider = provider.filter(|p| !p.is_empty());
        sqlx::query(
            r#"
            INSERT INTO token_events
                (session_id, ts, input_tokens, output_tokens, total_tokens, billed_total_tokens, model_id, provider,
                 cache_read_tokens, cache_creation_tokens, session_type)
            VALUES (?, CAST(strftime('%s', 'now') AS INTEGER), ?, ?, ?, ?, ?, ?, ?, ?,
                    (SELECT session_type FROM sessions WHERE id = ?))
            "#,
        )
        .bind(session_id)
        .bind(input.map(i64::from))
        .bind(output.map(i64::from))
        .bind(total)
        .bind(total)
        .bind(model)
        .bind(provider)
        .bind(cache_read.map(i64::from))
        .bind(cache_creation.map(i64::from))
        .bind(session_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_usage_event(&self, entry: UsageLedgerEntry) -> Result<bool> {
        if entry.event_key.trim().is_empty() {
            anyhow::bail!("usage event key must not be empty");
        }

        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;
        let model = entry.model_id.as_deref().filter(|model| !model.is_empty());
        let provider = entry
            .provider
            .as_deref()
            .filter(|provider| !provider.is_empty());
        let context_or_legacy_total = entry
            .current_total_tokens
            .map(i64::from)
            .or(entry.billed_total_tokens)
            .unwrap_or(0);
        let inserted = sqlx::query(
            r#"
            INSERT INTO token_events
                (session_id, ts, input_tokens, output_tokens, total_tokens, billed_total_tokens, model_id, provider,
                 cache_read_tokens, cache_creation_tokens, event_key, session_type)
            VALUES (?, CAST(strftime('%s', 'now') AS INTEGER), ?, ?, ?, ?, ?, ?, ?, ?, ?,
                    (SELECT session_type FROM sessions WHERE id = ?))
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(&entry.session_id)
        .bind(entry.input_tokens.map(i64::from))
        .bind(entry.output_tokens.map(i64::from))
        .bind(context_or_legacy_total)
        .bind(entry.billed_total_tokens)
        .bind(model)
        .bind(provider)
        .bind(entry.cache_read_tokens.map(i64::from))
        .bind(entry.cache_creation_tokens.map(i64::from))
        .bind(&entry.event_key)
        .bind(&entry.session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;

        if !inserted {
            tx.commit().await?;
            return Ok(false);
        }

        let update_current = entry.current_total_tokens.is_some()
            || entry.current_input_tokens.is_some()
            || entry.current_output_tokens.is_some();
        let input_delta = entry.input_tokens.map(i64::from);
        let output_delta = entry.output_tokens.map(i64::from);
        let updated = sqlx::query(
            r#"
            UPDATE sessions SET
                accumulated_total_tokens = CASE
                    WHEN ? IS NULL THEN accumulated_total_tokens
                    ELSE COALESCE(accumulated_total_tokens, 0) + ? END,
                accumulated_input_tokens = CASE
                    WHEN ? IS NULL THEN accumulated_input_tokens
                    ELSE COALESCE(accumulated_input_tokens, 0) + ? END,
                accumulated_output_tokens = CASE
                    WHEN ? IS NULL THEN accumulated_output_tokens
                    ELSE COALESCE(accumulated_output_tokens, 0) + ? END,
                total_tokens = CASE WHEN ? THEN ? ELSE total_tokens END,
                input_tokens = CASE WHEN ? THEN ? ELSE input_tokens END,
                output_tokens = CASE WHEN ? THEN ? ELSE output_tokens END,
                schedule_id = ?,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(entry.billed_total_tokens)
        .bind(entry.billed_total_tokens)
        .bind(input_delta)
        .bind(input_delta)
        .bind(output_delta)
        .bind(output_delta)
        .bind(update_current)
        .bind(entry.current_total_tokens)
        .bind(update_current)
        .bind(entry.current_input_tokens)
        .bind(update_current)
        .bind(entry.current_output_tokens)
        .bind(&entry.schedule_id)
        .bind(&entry.session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if updated != 1 {
            anyhow::bail!("session not found");
        }

        tx.commit().await?;
        Ok(true)
    }

    /// Per-model rollup for one session. NULL `model_id` groups as its own row
    /// (the caller surfaces it as "unknown").
    async fn get_session_model_usage(&self, session_id: &str) -> Result<Vec<ModelUsageRow>> {
        let pool = self.pool().await?;
        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
                .bind(session_id)
                .fetch_one(pool)
                .await?;
        if !exists {
            anyhow::bail!("session not found");
        }
        let rows = sqlx::query_as::<_, ModelUsageRow>(
            r#"
            SELECT model_id,
                   provider,
                   COALESCE(SUM(input_tokens), 0)  AS input_tokens,
                   COALESCE(SUM(output_tokens), 0) AS output_tokens,
                   CASE WHEN COUNT(billed_total_tokens) = COUNT(*)
                        THEN SUM(billed_total_tokens) END AS total_tokens,
                   CASE WHEN COUNT(cache_read_tokens) = COUNT(*)
                        THEN SUM(cache_read_tokens) END AS cache_read_tokens,
                   CASE WHEN COUNT(cache_creation_tokens) = COUNT(*)
                        THEN SUM(cache_creation_tokens) END AS cache_creation_tokens,
                   COUNT(*)                        AS turns
            FROM token_events
            WHERE session_id = ?1
            GROUP BY model_id, provider
            ORDER BY total_tokens DESC
            "#,
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Global per-model rollup over the inclusive `[from, to]` unix-second window,
    /// restricted to billable session types. Subagent calls are real provider
    /// spend even though their internal sessions stay hidden from session lists,
    /// and so is a deleted chat's: this reads [`BILLABLE_USAGE`] (F10).
    async fn get_model_usage(&self, from: i64, to: i64) -> Result<Vec<ModelUsageRow>> {
        let pool = self.pool().await?;
        let rows = sqlx::query_as::<_, ModelUsageRow>(&format!(
            r#"
            SELECT u.model_id,
                   u.provider,
                   COALESCE(SUM(u.input_tokens), 0)  AS input_tokens,
                   COALESCE(SUM(u.output_tokens), 0) AS output_tokens,
                   CASE WHEN SUM(u.billed_known) = SUM(u.turns)
                        THEN SUM(u.billed_total_tokens) END AS total_tokens,
                   CASE WHEN SUM(u.cache_read_known) = SUM(u.turns)
                        THEN SUM(u.cache_read_tokens) END AS cache_read_tokens,
                   CASE WHEN SUM(u.cache_creation_known) = SUM(u.turns)
                        THEN SUM(u.cache_creation_tokens) END AS cache_creation_tokens,
                   SUM(u.turns)                       AS turns
            FROM {BILLABLE_USAGE}
            WHERE u.ts >= ?1 AND u.ts <= ?2
            GROUP BY u.model_id, u.provider
            ORDER BY total_tokens DESC
            "#
        ))
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Queryable usage report over the inclusive `[from, to]` unix-second window.
    ///
    /// The SQL always groups at the finest `(day, model, provider)` grain; Rust
    /// then prices each grain row once and rolls it up into `group`. That order
    /// is what lets a `Day` bucket report a correct dollar cost even though the
    /// day mixes models at different prices. Reads [`BILLABLE_USAGE`], so a
    /// deleted chat's spend stays in the day it was spent on (F10).
    async fn get_usage_report(
        &self,
        from: i64,
        to: i64,
        group: UsageGroup,
    ) -> Result<Vec<UsageReportRow>> {
        let pool = self.pool().await?;
        let grain = sqlx::query_as::<_, UsageGrainRow>(&billable_usage_grain_sql(
            "u.day",
            "u.ts >= ?1 AND u.ts <= ?2",
        ))
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await?;
        let pricing = resolve_grain_pricing(&grain).await;
        Ok(rollup_report_with_pricing(&grain, group, &pricing))
    }

    /// Month-to-date (current local month) + all-time priced totals, over
    /// [`BILLABLE_USAGE`] — deleted chats' spend included, since the gauge is
    /// held against the provider's own meter (F10, issue #1).
    async fn get_usage_summary(&self) -> Result<UsageSummary> {
        let pool = self.pool().await?;

        // Per-model grain is required so each model prices at its own rate before
        // summing; `day` is unused here, so a constant keeps the shared struct.
        let mtd_grain = sqlx::query_as::<_, UsageGrainRow>(&billable_usage_grain_sql(
            "''",
            "substr(u.day, 1, 7) = strftime('%Y-%m', 'now', 'localtime')",
        ))
        .fetch_all(pool)
        .await?;

        let all_grain =
            sqlx::query_as::<_, UsageGrainRow>(&billable_usage_grain_sql("''", "1 = 1"))
                .fetch_all(pool)
                .await?;

        let month: String = sqlx::query_scalar("SELECT strftime('%Y-%m', 'now', 'localtime')")
            .fetch_one(pool)
            .await?;
        let mtd_pricing = resolve_grain_pricing(&mtd_grain).await;
        let all_pricing = resolve_grain_pricing(&all_grain).await;

        Ok(UsageSummary {
            month,
            month_to_date: totals_from_grain_with_pricing(&mtd_grain, &mtd_pricing),
            all_time: totals_from_grain_with_pricing(&all_grain, &all_pricing),
        })
    }

    async fn get_activity(&self, days: i64) -> Result<ActivityWindow> {
        let pool = self.pool().await?;
        let days = days.clamp(1, 371);
        // SQLite's `-N days` modifier takes a literal, so build it once.
        let window = format!("-{days} days");

        // Sessions started per LOCAL calendar day. `created_at` never moves, so a
        // day's session count is stable across renders — unlike `updated_at`.
        let session_rows = sqlx::query_as::<_, (String, i64)>(
            r#"
            SELECT date(created_at, 'localtime') AS day, COUNT(*) AS n
            FROM sessions
            WHERE session_type IN ('user', 'scheduled')
              AND created_at >= datetime('now', ?1)
            GROUP BY day
            "#,
        )
        .bind(&window)
        .fetch_all(pool)
        .await?;

        // Tokens of chats that still exist (F10), like the session and message
        // counts beside them: a deleted chat leaves all three series at once.
        // Its spend stays in the Usage panel; see [`BILLABLE_USAGE`].
        let token_rows = sqlx::query_as::<_, (String, i64, i64, i64, bool)>(
            r#"
            SELECT date(te.ts, 'unixepoch', 'localtime') AS day,
                   COALESCE(SUM(te.billed_total_tokens), 0) AS tokens,
                   COALESCE(SUM(te.input_tokens), 0)  AS input_tokens,
                   COALESCE(SUM(te.output_tokens), 0) AS output_tokens,
                   COUNT(te.billed_total_tokens) = COUNT(*) AS tokens_complete
            FROM token_events te
            JOIN sessions s ON s.id = te.session_id
            WHERE te.session_type IN ('user', 'scheduled')
              AND te.ts >= CAST(strftime('%s', 'now', ?1) AS INTEGER)
            GROUP BY day
            "#,
        )
        .bind(&window)
        .fetch_all(pool)
        .await?;

        // `messages.created_timestamp` is unix SECONDS (Message::new uses
        // `Utc::now().timestamp()`), not milliseconds.
        let message_rows = sqlx::query_as::<_, (String, i64)>(
            r#"
            SELECT date(m.created_timestamp, 'unixepoch', 'localtime') AS day, COUNT(*) AS n
            FROM messages m
            JOIN sessions s ON s.id = m.session_id
            WHERE s.session_type IN ('user', 'scheduled')
              AND m.created_timestamp >= CAST(strftime('%s', 'now', ?1) AS INTEGER)
            GROUP BY day
            "#,
        )
        .bind(&window)
        .fetch_all(pool)
        .await?;

        let bounds = sqlx::query_as::<_, (String, String)>(
            "SELECT date('now', ?1, 'localtime'), date('now', 'localtime')",
        )
        .bind(&window)
        .fetch_one(pool)
        .await?;

        Ok(build_activity_window(
            bounds.0,
            bounds.1,
            &session_rows,
            &token_rows,
            &message_rows,
        ))
    }

    async fn export_session(&self, id: &str) -> Result<String> {
        let mut session = self.get_session(id, true).await?;
        session
            .extension_data
            .redact_resolved_auth_material_for_export();
        if let Some(extensions) = session
            .workflow
            .as_mut()
            .and_then(|workflow| workflow.extensions.as_mut())
        {
            *extensions = extensions
                .iter()
                .map(ExtensionConfig::redacted_for_session_export)
                .collect();
        }
        serde_json::to_string_pretty(&session).map_err(Into::into)
    }

    async fn import_session(
        &self,
        session_manager: &SessionManager,
        json: &str,
    ) -> Result<Session> {
        let import: Session = serde_json::from_str(json)?;

        let session = self
            .create_session(
                import.working_dir.clone(),
                import.name.clone(),
                import.session_type,
            )
            .await?;

        // Parsed, but never authoritative for `public`. An imported transcript
        // of unknown provenance is sensitive: unlike migration, there is no
        // local evidence to reason from, so the imported field is read ONLY in
        // the raising direction (issue #56 §9.3 B1).
        //
        // ⚠ On today's two-element lattice that max COLLAPSES — `Private` is the
        // top, so the result is `Private` whatever the file said, and `imported`
        // decides nothing. Review flagged this as arithmetic that reads live and
        // is not, and it is written down rather than simplified away for one
        // reason: `max` is what the rule ACTUALLY is ("only ever raise"), so a
        // third tier above Private would be honoured here instead of silently
        // floored to Private by a hardcoded variant. The discriminating half of
        // the rule is therefore the one its test asserts — an imported
        // `"public"` cannot lower the row — and not the "raised by it" half,
        // which no value in this enum can exercise.
        let imported = import.privacy_tier;

        let mut builder = session_manager
            .update(&session.id)
            .raise_privacy(SessionClassification::Private.max(imported), "imported")
            .extension_data(import.extension_data)
            .total_tokens(import.total_tokens)
            .input_tokens(import.input_tokens)
            .output_tokens(import.output_tokens)
            .accumulated_total_tokens(import.accumulated_total_tokens)
            .accumulated_input_tokens(import.accumulated_input_tokens)
            .accumulated_output_tokens(import.accumulated_output_tokens)
            .schedule_id(import.schedule_id)
            .workflow(import.workflow)
            .user_workflow_values(import.user_workflow_values);

        if import.user_set_name {
            builder = builder.user_provided_name(import.name.clone());
        }

        builder.apply().await?;

        if let Some(conversation) = import.conversation {
            self.replace_conversation(&session.id, &conversation)
                .await?;
        }

        self.get_session(&session.id, true).await
    }

    /// Create a session derived from `source`, carrying everything a branch must
    /// inherit. The three copy paths (`copy_session`, `diverge_session_for_edit`,
    /// `diverge_session`) each hand-rolled their own builder, and none of them
    /// carried `provider_name`/`model_config`/`privacy_tier` — so a branch of a
    /// private chat resolved through `restore_provider_from_session`'s
    /// `Config::global()` fallback and ran private history on the user's default
    /// public model, with no prompt (issue #56 §9.3 B1).
    ///
    /// Callers add only their own extras (`user_provided_name`, `diverged_from`,
    /// `branch_point_msg_uid`) and their own conversation, so the carry-over
    /// cannot be missed by one of them.
    ///
    /// ⚠ **`raise_privacy` is called unconditionally, and on a PUBLIC source
    /// that writes a `privacy_reason` where the child previously had none.** The
    /// tier is untouched — a public source raises a public child to public — but
    /// the dominance `CASE` in [`SessionUpdateBuilder::apply`] guards both preserving
    /// arms on the row being already non-public, so the trailing `ELSE` fires
    /// and stamps `diverged:<parent>` on a public row. Harmless today:
    /// `privacy_reason` is audit and UX only, never read by a gate, and the
    /// value is true of the row it lands on.
    ///
    /// It stops being harmless at exactly the point that `apply`'s own note
    /// names — when §12.5 declassification starts leaving `declassified_by_user`
    /// in `privacy_reason` on a row it has just returned to public. A copy of a
    /// declassified chat would then erase that provenance. Whoever lands
    /// declassification has to decide there whether the `ELSE` arm preserves it
    /// the way the `mcp:` arm preserves its own; this call site is the one that
    /// will hit it, so it is named here as well as there.
    async fn create_derived_session(
        &self,
        session_manager: &SessionManager,
        source: &Session,
        new_name: String,
        reason: &str,
    ) -> Result<Session> {
        let new_session = self
            .create_session(source.working_dir.clone(), new_name, source.session_type)
            .await?;
        let mut update = session_manager
            .update(&new_session.id)
            .extension_data(source.extension_data.clone())
            .schedule_id(source.schedule_id.clone())
            .workflow(source.workflow.clone())
            .user_workflow_values(source.user_workflow_values.clone())
            .raise_privacy(source.privacy_tier, reason);
        // ⚠ The two provider setters take VALUES, not Options:
        // `provider_name(impl Into<String>)` and `model_config(ModelConfig)`
        // each wrap their argument as `Some(Some(v))`, because
        // `Option<Option<T>>` is how this builder distinguishes "leave alone"
        // from "set to NULL". Neither setter has an Option-taking variant, and
        // passing `source.model_config.clone()` straight in is a type error. A source
        // with no provider must leave the child's column untouched rather than
        // writing NULL over the default.
        if let Some(name) = source.provider_name.clone() {
            update = update.provider_name(name);
        }
        if let Some(cfg) = source.model_config.clone() {
            update = update.model_config(cfg);
        }
        update.apply().await?;
        self.get_session(&new_session.id, false).await
    }

    async fn copy_session(
        &self,
        session_manager: &SessionManager,
        session_id: &str,
        new_name: String,
    ) -> Result<Session> {
        let original_session = self.get_session(session_id, true).await?;

        let new_session = self
            .create_derived_session(
                session_manager,
                &original_session,
                new_name,
                &format!("diverged:{session_id}"),
            )
            .await?;

        if let Some(conversation) = original_session.conversation.as_ref() {
            self.replace_conversation(&new_session.id, conversation)
                .await?;
        }

        self.get_session(&new_session.id, true).await
    }

    async fn diverge_session_for_edit(
        &self,
        session_manager: &SessionManager,
        session_id: &str,
        timestamp: i64,
    ) -> Result<Session> {
        let original = self.get_session(session_id, true).await?;
        let new_name = self.compute_branch_name(&original).await?;
        let branch_point = original.conversation.as_ref().and_then(|conversation| {
            conversation
                .messages()
                .iter()
                .rfind(|message| message.created < timestamp)
                .and_then(|message| message.id.clone())
        });

        // The same carry-over `copy_session` performs, spelled here rather than
        // delegated, so no copy path can quietly stop using the shared helper
        // (`no_copy_path_hand_rolls_its_own_builder_any_more`).
        //
        // ⚠ This path used to call `copy_session`, which re-read the parent a
        // second time. It now branches from the SINGLE `original` snapshot read
        // above, so `branch_point` and the copied conversation can no longer
        // disagree — a message appended to the parent between the two former
        // reads used to land in the branch's conversation while sitting outside
        // the branch point computed from the earlier read. The deliberate
        // consequence is that such a message is no longer carried at all, which
        // is the consistent reading of "branch the conversation the caller
        // asked about".
        let new_session = self
            .create_derived_session(
                session_manager,
                &original,
                new_name.clone(),
                &format!("diverged:{session_id}"),
            )
            .await?;

        if let Some(conversation) = original.conversation.as_ref() {
            self.replace_conversation(&new_session.id, conversation)
                .await?;
        }

        session_manager
            .update(&new_session.id)
            .user_provided_name(new_name)
            .diverged_from(Some(session_id.to_string()))
            .branch_point_msg_uid(branch_point)
            .apply()
            .await?;

        self.truncate_conversation(&new_session.id, timestamp)
            .await?;
        self.get_session(&new_session.id, true).await
    }

    async fn diverge_session(
        &self,
        session_manager: &SessionManager,
        session_id: &str,
        custom_name: Option<String>,
        anchor_ms: Option<i64>,
        anchor_uid: Option<String>,
    ) -> Result<Session> {
        // Load original first (with conversation) so we can derive a name and
        // confirm it exists.
        let original = self.get_session(session_id, true).await?;

        let new_name = match custom_name {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => self.compute_branch_name(&original).await?,
        };

        // Build the branch conversation: the parent's history trimmed to end at
        // the last complete assistant answer (so a mid-generation diverge never
        // carries over an unanswered question or a dangling tool call). The
        // durable message id (`anchor_uid`) is preferred over the timestamp so a
        // divergence at one of two same-second messages does not over-truncate.
        //
        // The cut is resolved ONCE, here, and reused for the branch point below
        // — issue #167. Recording the *requested* anchor instead was a claim
        // nothing checked: an id `resolve_branch_anchor` refused (unknown, or
        // older than the timestamp beside it) was still written to
        // `branch_point_msg_uid`, so the row named a divergence point the branch
        // had deliberately not been cut at.
        let (branch_conversation, honoured_anchor) = original.conversation.as_ref().map_or_else(
            || (Conversation::default(), None),
            |c| branch_conversation_at(c, anchor_uid.as_deref(), anchor_ms),
        );

        // Record the divergence point: the anchor that was actually honoured,
        // else the id of the last message carried into the branch.
        let branch_point = honoured_anchor.or_else(|| {
            branch_conversation
                .messages()
                .last()
                .and_then(|m| m.id.clone())
        });

        // Mint the branch session with the shared carry-over (same as
        // copy_session, but this path writes the *trimmed* conversation rather
        // than the full one). This is the path `POST /sessions/{id}/diverge`
        // reaches — it does NOT go through `copy_session`.
        let new_session = self
            .create_derived_session(
                session_manager,
                &original,
                new_name.clone(),
                &format!("diverged:{session_id}"),
            )
            .await?;

        session_manager
            .update(&new_session.id)
            // Lock the computed/custom name (so the auto-namer never clobbers
            // the branch marker) and record the lineage pointer + divergence point.
            .user_provided_name(new_name)
            .diverged_from(Some(session_id.to_string()))
            .branch_point_msg_uid(branch_point)
            .apply()
            .await?;

        self.replace_conversation(&new_session.id, &branch_conversation)
            .await?;

        self.get_session(&new_session.id, true).await
    }

    /// Derive `"{base} (branch {N})"` for a diverged session. `base` is the
    /// parent's name with any `(branch K)` suffix stripped, falling back to a
    /// conversation-derived title when the parent's name is just a placeholder.
    /// `N` is the next free index across that base's branch family.
    async fn compute_branch_name(&self, original: &Session) -> Result<String> {
        let stripped = strip_branch_suffix(&original.name);
        let base = if !original.user_set_name && is_default_session_name(stripped) {
            let derived = original
                .conversation
                .as_ref()
                .map(SessionManager::fallback_session_name)
                .unwrap_or_default();
            if derived.trim().is_empty() {
                "Conversation".to_string()
            } else {
                derived
            }
        } else {
            stripped.to_string()
        };

        let next = self.count_branch_siblings(&base).await? + 1;
        Ok(format!("{base} (branch {next})"))
    }

    /// Count existing sessions named `"{base} (branch <digits>)"` so the next
    /// branch gets a unique index. The base is escaped for use in a SQL LIKE.
    async fn count_branch_siblings(&self, base: &str) -> Result<i64> {
        let pool = self.pool().await?;
        let pattern = format!("{} (branch %)", like_escape(base));
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sessions WHERE name LIKE ? ESCAPE '\\'",
        )
        .bind(pattern)
        .fetch_one(pool)
        .await?;
        Ok(count)
    }

    /// Delete every message of `session_id` at or after `timestamp`, together
    /// with the side state those rows owned, in ONE transaction.
    ///
    /// `bound` bounds the delete by rowid. `None` means "whatever exists when
    /// this transaction takes the write lock", which is the historical
    /// timestamp-only behaviour; `Some(basis)` restricts it to the rows that
    /// caller's own view covered, so an append that landed after that view was
    /// taken — necessarily newer, therefore necessarily inside an open-ended
    /// `created_timestamp >= ?` range — is NOT part of the tail being dropped.
    /// A `basis` from a previous incarnation of the id is refused: its rowids
    /// describe a conversation that no longer exists.
    ///
    /// Three things used to be skipped here that the rewrite path has always
    /// done, and all three are why this is a transaction rather than a
    /// statement:
    /// - the FTS recall mirror kept a row per deleted message forever (every
    ///   checkpoint restore leaked more; only an unrelated `INNER JOIN
    ///   messages` in the search query kept them from surfacing as hits),
    /// - the BR-7 payload of a deleted oversized tool response was stranded in
    ///   `message_blobs` with nothing referencing it,
    /// - `sessions.updated_at` never moved, so an edited or restored session
    ///   sorted as untouched in the `ORDER BY updated_at DESC` list.
    ///
    /// Returns how many message rows were removed.
    async fn truncate_conversation_inner(
        &self,
        session_id: &str,
        timestamp: i64,
        bound: Option<ConversationRevision>,
    ) -> Result<TruncateOutcome> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await?;

        // LOAD-BEARING FIRST STATEMENT, AND IT MUST BE A WRITE. DO NOT REORDER.
        // Identical reasoning to `replace_conversation_inner`: a deferred
        // transaction that reads first pins a WAL snapshot and the later
        // upgrade to a writer fails SQLITE_BUSY_SNAPSHOT immediately, bypassing
        // the busy timeout. Opening with the `updated_at` bump takes the write
        // lock up front, which is also what makes the row set selected below
        // identical to the row set deleted afterwards.
        let touched = sqlx::query("UPDATE sessions SET updated_at = datetime('now') WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        if touched.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(TruncateOutcome::SessionNotFound);
        }

        let upper = match bound {
            // Row identity is checked UNDER THE LOCK, alongside the watermark it
            // qualifies — reading it from the pool first would leave a window in
            // which the id is re-issued between the check and the delete.
            Some(basis) => {
                if Self::read_incarnation(&mut tx, session_id).await? != basis.incarnation {
                    tx.rollback().await?;
                    return Ok(TruncateOutcome::Stale);
                }
                basis.max_rowid
            }
            // Read under the lock, so it names exactly the rows that exist now.
            None => {
                sqlx::query_scalar::<_, i64>(
                    "SELECT IFNULL(MAX(id), 0) FROM messages WHERE session_id = ?",
                )
                .bind(session_id)
                .fetch_one(&mut *tx)
                .await?
            }
        };

        const DOOMED: &str =
            "FROM messages WHERE session_id = ? AND created_timestamp >= ? AND id <= ?";

        // The doomed rows' payload references, captured before they go. Nothing
        // can interleave under the write lock, so re-evaluating the same
        // predicate in the DELETE below selects exactly this set — which is why
        // there is no need to marshal thousands of ids through an IN list.
        let doomed = sqlx::query_scalar::<_, String>(&format!("SELECT content_json {DOOMED}"))
            .bind(session_id)
            .bind(timestamp)
            .bind(upper)
            .fetch_all(&mut *tx)
            .await?;
        if doomed.is_empty() {
            tx.commit().await?;
            return Ok(TruncateOutcome::Truncated { removed: 0 });
        }
        let dropped_a_stub = doomed
            .iter()
            .any(|content_json| message_blobs::content_json_has_stub(content_json));

        if Self::messages_fts_exists(&mut *tx).await {
            sqlx::query(&format!(
                "DELETE FROM messages_fts WHERE session_id = ? \
                 AND message_id IN (SELECT id {DOOMED})"
            ))
            .bind(session_id)
            .bind(session_id)
            .bind(timestamp)
            .bind(upper)
            .execute(&mut *tx)
            .await?;
        }

        let removed = sqlx::query(&format!("DELETE {DOOMED}"))
            .bind(session_id)
            .bind(timestamp)
            .bind(upper)
            .execute(&mut *tx)
            .await?
            .rows_affected() as usize;

        // Only when a dropped row actually carried a stub — the same "pay for
        // the scan only when there is something to sweep" discipline as the
        // read path.
        if dropped_a_stub {
            let survivors = sqlx::query_scalar::<_, String>(
                "SELECT content_json FROM messages WHERE session_id = ?",
            )
            .bind(session_id)
            .fetch_all(&mut *tx)
            .await?;
            let mut live_blob_uids: Vec<String> = Vec::new();
            for content_json in &survivors {
                if !message_blobs::content_json_has_stub(content_json) {
                    continue;
                }
                let content: Vec<MessageContent> = serde_json::from_str(content_json)?;
                live_blob_uids.extend(message_blobs::referenced_uids(&content));
            }
            Self::sweep_orphan_blobs(&mut tx, session_id, &live_blob_uids).await?;
        }

        tx.commit().await?;
        Ok(TruncateOutcome::Truncated { removed })
    }

    async fn truncate_conversation(&self, session_id: &str, timestamp: i64) -> Result<()> {
        self.truncate_conversation_inner(session_id, timestamp, None)
            .await
            .map(|_| ())
    }

    /// See [`SessionManager::truncate_conversation_bounded`].
    async fn truncate_conversation_bounded(
        &self,
        session_id: &str,
        timestamp: i64,
        basis: ConversationRevision,
    ) -> Result<TruncateOutcome> {
        self.truncate_conversation_inner(session_id, timestamp, Some(basis))
            .await
    }

    // BR-43 shadow-git checkpoints: the `checkpoints` side-table CRUD. Kept here
    // (rather than the `checkpoint` module) because `SessionStorage` owns the
    // SQLite pool; `CheckpointManager` calls these through `SessionManager`.

    async fn insert_checkpoint(&self, rec: &crate::checkpoint::CheckpointRecord) -> Result<()> {
        let pool = self.pool().await?;
        let changed = serde_json::to_string(&rec.changed_paths)?;
        sqlx::query(
            r#"INSERT INTO checkpoints
                (id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha, changed_paths_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&rec.id)
        .bind(&rec.session_id)
        .bind(rec.turn_index)
        .bind(rec.anchor_ts)
        .bind(rec.kind.as_str())
        .bind(&rec.commit_sha)
        .bind(&rec.tree_sha)
        .bind(changed)
        .bind(&rec.created_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn list_checkpoints(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::checkpoint::CheckpointRecord>> {
        let pool = self.pool().await?;
        let rows = sqlx::query_as::<_, CheckpointRow>(
            "SELECT id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha, changed_paths_json, created_at
             FROM checkpoints WHERE session_id = ? ORDER BY turn_index DESC",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(CheckpointRow::into_record).collect()
    }

    /// The highest-`turn_index` checkpoint, for the next ordinal + `tree_sha`
    /// dedup baseline.
    async fn last_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::checkpoint::CheckpointRecord>> {
        let pool = self.pool().await?;
        let row = sqlx::query_as::<_, CheckpointRow>(
            "SELECT id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha, changed_paths_json, created_at
             FROM checkpoints WHERE session_id = ? ORDER BY turn_index DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(pool)
        .await?;
        row.map(CheckpointRow::into_record).transpose()
    }

    async fn get_checkpoint(
        &self,
        session_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<crate::checkpoint::CheckpointRecord>> {
        let pool = self.pool().await?;
        let row = sqlx::query_as::<_, CheckpointRow>(
            "SELECT id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha, changed_paths_json, created_at
             FROM checkpoints WHERE session_id = ? AND id = ?",
        )
        .bind(session_id)
        .bind(checkpoint_id)
        .fetch_optional(pool)
        .await?;
        row.map(CheckpointRow::into_record).transpose()
    }

    async fn delete_checkpoints(&self, session_id: &str) -> Result<()> {
        let pool = self.pool().await?;
        sqlx::query("DELETE FROM checkpoints WHERE session_id = ?")
            .bind(session_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn search_chat_history(
        &self,
        query: &str,
        limit: Option<usize>,
        after_date: Option<chrono::DateTime<chrono::Utc>>,
        before_date: Option<chrono::DateTime<chrono::Utc>>,
        exclude_session_id: Option<String>,
        // Issue #56 Gate D and DR-26 / Task 50 Step 3: both axes together — see
        // `chat_history_search::SearchReach`.
        reach: crate::session::chat_history_search::SearchReach,
    ) -> Result<crate::session::chat_history_search::ChatRecallResults> {
        use crate::session::chat_history_search::ChatHistorySearch;

        let pool = self.pool().await?;
        ChatHistorySearch::new(
            pool,
            query,
            limit,
            after_date,
            before_date,
            exclude_session_id,
            reach,
        )
        .execute()
        .await
    }
}

#[cfg(test)]
mod blob_tests {
    //! BR-7: externalizing an oversized tool result out of `content_json`.
    //!
    //! These drive the storage layer directly (`get_conversation_inner`) rather
    //! than through the `BIOROUTER_SESSION_BLOB_LAZY_LOAD` env var, so both read
    //! modes are covered deterministically under a parallel test runner.

    use super::*;
    use crate::conversation::message::{Message, ToolResponse};
    use rmcp::model::{CallToolResult, Content};
    use tempfile::TempDir;

    /// Comfortably over the 64 KiB default threshold.
    fn huge(marker: &str) -> String {
        (0..3_000)
            .map(|i| format!("{marker} row {i} of a very large tool result\n"))
            .collect()
    }

    fn tool_response_message(call_id: &str, text: String) -> Message {
        Message::assistant().with_content(MessageContent::ToolResponse(ToolResponse {
            id: call_id.to_string(),
            tool_result: Ok(CallToolResult {
                content: vec![Content::text(text)],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
            metadata: None,
        }))
    }

    fn response_text(conv: &Conversation, idx: usize) -> String {
        let MessageContent::ToolResponse(response) = &conv.messages()[idx].content[0] else {
            panic!("expected a tool response");
        };
        response.tool_result.as_ref().unwrap().content[0]
            .as_text()
            .unwrap()
            .text
            .clone()
    }

    async fn stored_content_json(sm: &SessionManager, session_id: &str) -> String {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_scalar::<_, String>(
            "SELECT content_json FROM messages WHERE session_id = ? ORDER BY id LIMIT 1",
        )
        .bind(session_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn blob_count(sm: &SessionManager, session_id: &str) -> i64 {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM message_blobs WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn session(sm: &SessionManager) -> String {
        sm.create_session(
            PathBuf::from("/tmp"),
            "blobs".to_string(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id
    }

    /// #41: replaying an OVERSIZED message must be as idempotent as any other
    /// replay. The probe used to re-externalize the candidate — minting a
    /// fresh blob uid on every comparison — so a large-message replay never
    /// compared equal to its stored row and was re-inserted (duplicated)
    /// under a re-minted uid. The comparison now hydrates the stored stubs
    /// and compares pre-externalization payloads, which is stable.
    #[tokio::test]
    async fn an_oversized_replay_is_idempotent_not_a_reminted_duplicate() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        let message = tool_response_message("call_1", huge("r")).with_id("big-uid");
        let first = sm.add_message(&id, &message).await.unwrap();
        assert_eq!(first, "big-uid");

        let replay = sm
            .add_message(&id, &message)
            .await
            .expect("an oversized replay must be idempotent success");
        assert_eq!(
            replay, "big-uid",
            "the replay returns the SAME uid, with no re-mint for identical content"
        );

        let loaded = sm.get_session(&id, true).await.unwrap();
        assert_eq!(
            loaded.conversation.unwrap().len(),
            1,
            "an oversized replay must not create a duplicate row"
        );
        assert_eq!(
            blob_count(&sm, &id).await,
            1,
            "an oversized replay must not spill a duplicate blob"
        );
    }

    /// The core of BR-7: the oversized payload leaves `content_json` for the side
    /// table, and the default (hydrating) read puts it back byte for byte — so
    /// no existing consumer can tell the difference.
    #[tokio::test]
    async fn an_oversized_tool_result_is_externalized_and_hydrated_back_exactly() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        let payload = huge("a");
        sm.add_message(&id, &tool_response_message("call_1", payload.clone()))
            .await
            .unwrap();

        // The message row is now tiny and carries only a stub.
        let row = stored_content_json(&sm, &id).await;
        assert!(
            row.len() < payload.len() / 10,
            "the stored row should be a stub, not the payload ({} vs {})",
            row.len(),
            payload.len()
        );
        assert!(message_blobs::content_json_has_stub(&row));
        assert_eq!(blob_count(&sm, &id).await, 1);

        // ...and the default read is indistinguishable from before.
        let conv = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(response_text(&conv, 0), payload);
    }

    #[tokio::test]
    async fn an_ordinary_tool_result_still_stores_inline() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        sm.add_message(&id, &tool_response_message("call_1", "small result".into()))
            .await
            .unwrap();

        assert!(stored_content_json(&sm, &id).await.contains("small result"));
        assert_eq!(blob_count(&sm, &id).await, 0);
    }

    /// The lazy read: the model gets the stub (preview + handle) and pulls the
    /// payload back only when it asks, through `get_message_blob`.
    #[tokio::test]
    async fn the_lazy_read_keeps_the_stub_and_the_handle_resolves() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        let payload = huge("b");
        sm.add_message(&id, &tool_response_message("call_1", payload.clone()))
            .await
            .unwrap();

        let conv = sm
            .storage()
            .get_conversation_inner(&id, false)
            .await
            .unwrap();
        let stub = response_text(&conv, 0);
        assert!(stub.len() < payload.len() / 10);
        assert!(stub.contains("platform__read_session_blob"));
        assert!(stub.contains("b row 0 of a very large tool result"));

        let uid = message_blobs::blob_uid_of(&stub).expect("the stub names a blob");
        assert_eq!(
            sm.get_message_blob(&id, uid).await.unwrap(),
            Some(payload.clone())
        );

        // A handle is scoped to its session: another session cannot read it.
        let other = session(&sm).await;
        assert_eq!(sm.get_message_blob(&other, uid).await.unwrap(), None);
    }

    /// Compaction/edit rewrites the whole conversation. Kept messages must keep
    /// their payload, and a dropped message's blob must not linger — otherwise
    /// BR-7 would just move the bloat from one table to another.
    #[tokio::test]
    async fn a_conversation_rewrite_keeps_live_blobs_and_sweeps_dropped_ones() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        let kept = huge("keep");
        let dropped = huge("drop");
        sm.add_message(&id, &tool_response_message("call_1", kept.clone()))
            .await
            .unwrap();
        sm.add_message(&id, &tool_response_message("call_2", dropped))
            .await
            .unwrap();
        assert_eq!(blob_count(&sm, &id).await, 2);

        // Rewrite with only the first message — exactly what compaction does.
        let full = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let compacted = Conversation::new_unvalidated(vec![full.messages()[0].clone()]);
        sm.replace_conversation(&id, &compacted).await.unwrap();

        assert_eq!(blob_count(&sm, &id).await, 1);
        let conv = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(conv.messages().len(), 1);
        assert_eq!(response_text(&conv, 0), kept);
    }

    /// #51 W4: truncation drops message rows too, so it owes the side table the
    /// same sweep a rewrite does. Without it a checkpoint restore or a message
    /// edit strands every externalized payload behind the cut — megabytes per
    /// oversized tool result, kept alive by nothing and reachable by nobody.
    #[tokio::test]
    async fn a_truncation_sweeps_the_blobs_it_orphaned() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        let kept = huge("keep");
        let mut first = tool_response_message("call_1", kept.clone());
        first.created = 100;
        sm.add_message(&id, &first).await.unwrap();
        let mut dropped = tool_response_message("call_2", huge("drop"));
        dropped.created = 500;
        sm.add_message(&id, &dropped).await.unwrap();
        assert_eq!(blob_count(&sm, &id).await, 2);

        sm.truncate_conversation(&id, 500).await.unwrap();

        assert_eq!(
            blob_count(&sm, &id).await,
            1,
            "the truncated message's payload must go with it"
        );
        let conv = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(conv.messages().len(), 1);
        assert_eq!(
            response_text(&conv, 0),
            kept,
            "...and the surviving message's payload must still hydrate"
        );
    }

    /// A message RECOVERED by the freshness guard carries its externalized
    /// payload with it. The recovered rows must join the merged list BEFORE the
    /// blob accounting runs — build the list afterwards and `sweep_orphan_blobs`
    /// deletes the payload out from under a message that survives, leaving a
    /// dangling stub. Silent, and only visible on the next read.
    #[tokio::test]
    async fn preserving_tail_keeps_blobs_of_recovered_messages() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        sm.add_message(&id, &Message::user().with_text("prompt"))
            .await
            .unwrap();
        let (snap, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
        let known = snap.conversation.unwrap();

        // A concurrent writer appends an OVERSIZED tool result, which is
        // externalized into `message_blobs`.
        let payload = huge("recovered");
        sm.add_message(&id, &tool_response_message("call_1", payload.clone()))
            .await
            .unwrap();
        assert_eq!(blob_count(&sm, &id).await, 1);

        // ...and the caller compacts what it saw away.
        let replacement = Conversation::new_unvalidated(vec![Message::user().with_text("summary")]);
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            ReplaceOutcome::ReplacedPreservingTail { preserved: 1 }
        );

        // Exactly one blob row: spared, not swept, and not duplicated by a
        // hydrate-then-re-externalize round trip.
        assert_eq!(blob_count(&sm, &id).await, 1);
        let conv = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(conv.messages().len(), 2);
        assert_eq!(
            response_text(&conv, 1),
            payload,
            "the recovered message must still hydrate to its full payload"
        );
    }

    /// The conversation RETURNED by a tail-preserving rewrite is what the live
    /// turn adopts (`conversation = stored`) and what the UI is told to render
    /// (`HistoryReplaced`). It must therefore carry the recovered tail's real
    /// payload, not the storage-shaped stub the rewrite (correctly) re-inserted.
    ///
    /// Before this was fixed the returned tail was ~1 KB of stub while the row
    /// on disk held the full 130 KB — so the model spent the rest of the turn
    /// reasoning over a placeholder, and only a reload healed the transcript.
    #[tokio::test]
    async fn preserving_tail_returns_a_hydrated_conversation_to_the_caller() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        sm.add_message(&id, &Message::user().with_text("prompt"))
            .await
            .unwrap();
        let (snap, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
        let known = snap.conversation.unwrap();

        // A concurrent writer appends an oversized tool result while the
        // summarizer runs; it is externalized on the way in.
        let payload = huge("recovered");
        sm.add_message(&id, &tool_response_message("call_1", payload.clone()))
            .await
            .unwrap();

        let replacement = Conversation::new_unvalidated(vec![Message::user().with_text("summary")]);
        let (outcome, returned) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            ReplaceOutcome::ReplacedPreservingTail { preserved: 1 }
        );

        assert_eq!(returned.messages().len(), 2);
        assert_eq!(
            response_text(&returned, 1),
            payload,
            "the RETURNED tail must carry the payload, not the stub"
        );

        // And hydrating the returned copy must not have disturbed storage: one
        // blob, still exactly one, and a re-read still agrees.
        assert_eq!(blob_count(&sm, &id).await, 1);
        let reread = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(response_text(&reread, 1), payload);
    }

    /// A rewrite of a *lazily* loaded conversation carries stubs, not payloads.
    /// The stub must keep pointing at a live blob — the sweep must not mistake a
    /// surviving handle for an orphan and delete the payload out from under it.
    #[tokio::test]
    async fn rewriting_a_lazily_loaded_conversation_does_not_lose_the_payload() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        let payload = huge("c");
        sm.add_message(&id, &tool_response_message("call_1", payload.clone()))
            .await
            .unwrap();

        let lazy = sm
            .storage()
            .get_conversation_inner(&id, false)
            .await
            .unwrap();
        sm.replace_conversation(&id, &lazy).await.unwrap();

        assert_eq!(blob_count(&sm, &id).await, 1);
        let conv = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(response_text(&conv, 0), payload);
    }

    /// Diverging copies the parent's messages into the child. When those messages
    /// are stubs (lazy mode), the child must end up owning its own copy of the
    /// payload, so the two sessions' lifetimes are independent.
    #[tokio::test]
    async fn a_branch_that_inherits_a_stub_adopts_the_payload() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let parent = session(&sm).await;
        let child = session(&sm).await;

        let payload = huge("d");
        sm.add_message(&parent, &tool_response_message("call_1", payload.clone()))
            .await
            .unwrap();

        // The parent's history as a lazy reader sees it: stubs.
        let lazy = sm
            .storage()
            .get_conversation_inner(&parent, false)
            .await
            .unwrap();
        sm.replace_conversation(&child, &lazy).await.unwrap();
        assert_eq!(blob_count(&sm, &child).await, 1);

        // Deleting the parent leaves the branch intact.
        sm.delete_session(&parent).await.unwrap();
        let conv = sm
            .get_session(&child, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(response_text(&conv, 0), payload);
    }

    #[tokio::test]
    async fn deleting_a_session_takes_its_blobs_with_it() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = session(&sm).await;

        sm.add_message(&id, &tool_response_message("call_1", huge("e")))
            .await
            .unwrap();
        assert_eq!(blob_count(&sm, &id).await, 1);

        sm.delete_session(&id).await.unwrap();
        assert_eq!(blob_count(&sm, &id).await, 0);
    }

    /// The production upgrade path: a v15 DB gains `message_blobs` and keeps
    /// every inline message it already had (they are never rewritten).
    #[tokio::test]
    async fn migrates_v15_db_to_v16_message_blobs() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();

            sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)")
                .execute(&pool).await.unwrap();
            for v in 1..=15 {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(v)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query(
                r#"CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
                    user_set_name BOOLEAN DEFAULT FALSE, session_type TEXT NOT NULL DEFAULT 'user', working_dir TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    extension_data TEXT DEFAULT '{}', total_tokens INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                    accumulated_total_tokens INTEGER, accumulated_input_tokens INTEGER, accumulated_output_tokens INTEGER,
                    schedule_id TEXT, workflow_json TEXT, user_workflow_values_json TEXT, provider_name TEXT,
                    model_config_json TEXT, diverged_from TEXT, external_key TEXT, branch_point_msg_uid TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query(
                r#"CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
                    role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL,
                    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, tokens INTEGER, metadata_json TEXT, msg_uid TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            SessionStorage::create_usage_schema(&pool).await.unwrap();
            sqlx::query(MESSAGES_FTS_DDL).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO sessions (id, name, working_dir) VALUES ('20240101_1', 'old', '/tmp/old')")
                .execute(&pool).await.unwrap();
            // A pre-v16 message keeps its content inline, forever.
            let legacy = serde_json::to_string(&vec![MessageContent::text("kept inline")]).unwrap();
            sqlx::query("INSERT INTO messages (session_id, role, content_json, created_timestamp, msg_uid) VALUES ('20240101_1', 'user', ?, 1, 'm1')")
                .bind(&legacy)
                .execute(&pool).await.unwrap();
            pool.close().await;
        }

        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let pool = sm.storage().pool().await.unwrap();
        assert_eq!(
            SessionStorage::get_schema_version(pool).await.unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        // The new table exists...
        assert_eq!(blob_count(&sm, "20240101_1").await, 0);
        // ...and the legacy message is untouched.
        let conv = sm
            .get_session("20240101_1", true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(conv.messages()[0].as_concat_text(), "kept inline");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use tempfile::TempDir;

    const NUM_CONCURRENT_SESSIONS: i32 = 10;

    /// Issue #56 — the export gate, at every corner.
    ///
    /// ⚠ These drive the REAL decision path (`SessionManager::authorize_export`)
    /// against a scratch store, never a re-derivation of the rule. The one thing
    /// they cannot drive is the platform dialog: `authenticate_export` raises a
    /// real Touch ID / polkit / Windows Hello prompt and there is no test seam
    /// for it here, so the `Authorized` arm is reached with an
    /// `ExportAuthorization` minted by `for_test` — which is exactly what
    /// `privacy::declassify`'s own tests do with `SystemAuthorization`.
    mod export_gate {
        use super::*;
        use crate::privacy::ProviderTier;

        async fn private_chat(sm: &SessionManager) -> String {
            let s = sm
                .create_session(
                    std::env::temp_dir(),
                    "a cohort chat".to_string(),
                    SessionType::User,
                )
                .await
                .unwrap();
            sm.add_message(&s.id, &Message::user().with_text("patient MRN 12345"))
                .await
                .unwrap();
            sm.update(&s.id)
                .provider_name("versa_azure")
                .raise_privacy(SessionClassification::Private, "mcp:ucsfomopagent")
                .apply()
                .await
                .unwrap();
            s.id
        }

        async fn public_chat(sm: &SessionManager) -> String {
            let s = sm
                .create_session(
                    std::env::temp_dir(),
                    "a public chat".to_string(),
                    SessionType::User,
                )
                .await
                .unwrap();
            sm.add_message(&s.id, &Message::user().with_text("hello"))
                .await
                .unwrap();
            s.id
        }

        async fn export_rows(
            sm: &SessionManager,
            session_id: &str,
        ) -> Vec<(String, String, String)> {
            let pool = sm.storage.pool().await.unwrap();
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT from_classification, to_classification, reason FROM \
                 classification_audit WHERE session_id = ?1 ORDER BY id",
            )
            .bind(session_id)
            .fetch_all(pool)
            .await
            .unwrap()
        }

        /// The headline: a public-capability caller cannot take a private
        /// transcript out of the store, and nothing is recorded when it tries.
        #[tokio::test]
        async fn a_public_caller_may_not_export_a_private_chat() {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            let id = private_chat(&sm).await;

            assert_eq!(
                sm.authorize_export(&id, ProviderTier::Public, None)
                    .await
                    .unwrap(),
                ExportDecision::CapabilityRequired
            );
            assert!(
                export_rows(&sm, &id).await.is_empty(),
                "a refused export must leave no ledger row"
            );
        }

        /// …and neither may a private-capability caller that the operating
        /// system has not authenticated. The two proofs answer different
        /// questions and neither substitutes for the other.
        #[tokio::test]
        async fn private_capability_alone_is_not_enough() {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            let id = private_chat(&sm).await;

            assert_eq!(
                sm.authorize_export(&id, ProviderTier::Private, None)
                    .await
                    .unwrap(),
                ExportDecision::SystemAuthenticationRequired
            );
            assert!(export_rows(&sm, &id).await.is_empty());
        }

        /// DR-20 point 4: an authentication covers the chats its dialog NAMED
        /// and no others. A proof minted for a different chat is not a proof.
        #[tokio::test]
        async fn an_authorization_for_another_chat_does_not_cover_this_one() {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            let id = private_chat(&sm).await;
            let elsewhere = ExportAuthorization::for_test(&["20990101_000000".to_string()]);

            assert_eq!(
                sm.authorize_export(&id, ProviderTier::Private, Some(&elsewhere))
                    .await
                    .unwrap(),
                ExportDecision::SystemAuthenticationRequired
            );
            assert!(export_rows(&sm, &id).await.is_empty());
        }

        /// The permitted path writes exactly one row — and the row says
        /// `private → private`, because the ruling is that the chat STAYS
        /// private. Anything that reads this table for "was declassified" keys
        /// on `to_classification = 'public'`, so an export can never be
        /// mistaken for one.
        #[tokio::test]
        async fn the_permitted_export_is_recorded_and_does_not_declassify() {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            let id = private_chat(&sm).await;
            let granted = ExportAuthorization::for_test(std::slice::from_ref(&id));

            assert_eq!(
                sm.authorize_export(&id, ProviderTier::Private, Some(&granted))
                    .await
                    .unwrap(),
                ExportDecision::Authorized
            );

            let rows = export_rows(&sm, &id).await;
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0],
                (
                    SessionClassification::PRIVATE_SQL.to_string(),
                    SessionClassification::PRIVATE_SQL.to_string(),
                    EXPORTED_BY_USER.to_string()
                )
            );

            // The ratchet is untouched: exporting is not a way to declassify.
            let row = sm.get_session(&id, false).await.unwrap();
            assert_eq!(row.privacy_tier, SessionClassification::Private);
            assert_eq!(row.privacy_reason.as_deref(), Some("mcp:ucsfomopagent"));
        }

        /// A public chat costs nothing: no prompt, no record, no copy. A gate
        /// that fired on every export is one people route around.
        #[tokio::test]
        async fn a_public_chat_is_unrestricted_and_leaves_no_row() {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            let id = public_chat(&sm).await;

            assert_eq!(
                sm.authorize_export(&id, ProviderTier::Public, None)
                    .await
                    .unwrap(),
                ExportDecision::Unrestricted
            );
            assert!(export_rows(&sm, &id).await.is_empty());
        }

        /// An id that names nothing is answered as such rather than as a
        /// refusal: there is no transcript to protect and no row to write.
        #[tokio::test]
        async fn an_unknown_session_is_reported_as_missing() {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            assert_eq!(
                sm.authorize_export("20990101_000000", ProviderTier::Private, None)
                    .await
                    .unwrap(),
                ExportDecision::SessionNotFound
            );
        }

        /// The copy is about the FILE, not about the chat — the failure it
        /// exists to prevent is the user believing the exported markdown
        /// inherits the chat's protection.
        #[test]
        fn the_notice_says_the_file_is_not_protected_and_the_chat_is_unchanged() {
            assert!(EXPORT_NOT_PROTECTED.contains("NOT protected"));
            assert!(EXPORT_NOT_PROTECTED.contains("stays private"));
            // …and it never claims the export changed the chat's tier.
            assert!(!EXPORT_NOT_PROTECTED.contains("declassif"));
        }
    }

    /// Issue #56 / BR-71: a subagent that produced nothing was invisible to the
    /// one listing that can show subagent runs at all.
    ///
    /// The regression is in SQL, not in rendering: `list_sessions_by_types`
    /// INNER JOINs `messages`, so a childless row is not returned. Both halves
    /// are asserted, because an assertion on the new entry point alone would
    /// pass just as well if the old one had been widened too — and widening it
    /// would put "Untitled chat" placeholders back in the desktop sidebar.
    #[tokio::test]
    async fn an_empty_subagent_is_listed_only_by_the_include_empty_query() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());

        let silent = sm
            .create_session(
                std::env::temp_dir(),
                "a child that produced nothing".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let spoke = sm
            .create_session(
                std::env::temp_dir(),
                "a child that spoke".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        sm.add_message(&spoke.id, &Message::user().with_text("i did some work"))
            .await
            .unwrap();

        let historical: Vec<String> = sm
            .list_sessions_by_types(&[SessionType::SubAgent])
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            historical,
            vec![spoke.id.clone()],
            "the sidebar query must keep hiding message-less rows"
        );

        let widened = sm
            .list_sessions_by_types_including_empty(&[SessionType::SubAgent])
            .await
            .unwrap();
        let ids: std::collections::BTreeSet<String> =
            widened.iter().map(|s| s.id.clone()).collect();
        assert!(
            ids.contains(&silent.id),
            "the childless subagent is still invisible: {ids:?}"
        );
        assert!(ids.contains(&spoke.id));
        // `COUNT(m.id)` ignores NULLs, so the LEFT JOIN must report 0 rather
        // than the 1 a naive count of joined rows would give.
        let silent_row = widened.iter().find(|s| s.id == silent.id).unwrap();
        assert_eq!(silent_row.message_count, 0);
        let spoke_row = widened.iter().find(|s| s.id == spoke.id).unwrap();
        assert_eq!(spoke_row.message_count, 1);
    }

    // #44 — the atomic empty-chat-only working-dir update. The emptiness check
    // lives in the UPDATE's own WHERE clause, so "insert a message between the
    // check and the write" is impossible by construction; these tests pin the
    // SQL path's three outcomes and the unguarded escape hatch.
    mod working_dir_guard {
        use super::*;
        use crate::session::session_manager::WorkingDirUpdate;

        #[tokio::test]
        async fn updates_an_empty_session_and_persists() {
            let store = TempDir::new().unwrap();
            let sm = SessionManager::new(store.path().to_path_buf());
            let session = sm
                .create_session(PathBuf::from("/tmp/old"), "s".into(), SessionType::User)
                .await
                .unwrap();

            let outcome = sm
                .try_update_working_dir_if_empty(&session.id, PathBuf::from("/tmp/new"))
                .await
                .unwrap();
            assert_eq!(outcome, WorkingDirUpdate::Updated);

            let reloaded = sm.get_session(&session.id, false).await.unwrap();
            assert_eq!(reloaded.working_dir, PathBuf::from("/tmp/new"));
        }

        #[tokio::test]
        async fn refuses_once_any_message_exists() {
            let store = TempDir::new().unwrap();
            let sm = SessionManager::new(store.path().to_path_buf());
            let session = sm
                .create_session(PathBuf::from("/tmp/old"), "s".into(), SessionType::User)
                .await
                .unwrap();
            sm.add_message(&session.id, &Message::user().with_text("hello"))
                .await
                .unwrap();

            let outcome = sm
                .try_update_working_dir_if_empty(&session.id, PathBuf::from("/tmp/new"))
                .await
                .unwrap();
            assert_eq!(outcome, WorkingDirUpdate::RefusedNotEmpty);

            // The refused update must not have touched the row.
            let reloaded = sm.get_session(&session.id, false).await.unwrap();
            assert_eq!(reloaded.working_dir, PathBuf::from("/tmp/old"));
        }

        #[tokio::test]
        async fn reports_a_missing_session_without_writing() {
            let store = TempDir::new().unwrap();
            let sm = SessionManager::new(store.path().to_path_buf());

            let outcome = sm
                .try_update_working_dir_if_empty("no_such_session", PathBuf::from("/tmp/new"))
                .await
                .unwrap();
            assert_eq!(outcome, WorkingDirUpdate::SessionNotFound);
        }

        #[tokio::test]
        async fn force_update_bypasses_the_guard_for_shell_following() {
            let store = TempDir::new().unwrap();
            let sm = SessionManager::new(store.path().to_path_buf());
            let session = sm
                .create_session(PathBuf::from("/tmp/old"), "s".into(), SessionType::User)
                .await
                .unwrap();
            sm.add_message(&session.id, &Message::user().with_text("hello"))
                .await
                .unwrap();

            // The `biorouter term run` shell-following path may move the dir
            // mid-conversation; nothing else may.
            sm.force_update_working_dir_unguarded(&session.id, PathBuf::from("/tmp/new"))
                .await
                .unwrap();

            let reloaded = sm.get_session(&session.id, false).await.unwrap();
            assert_eq!(reloaded.working_dir, PathBuf::from("/tmp/new"));
        }
    }

    /// BR-71: `extension_data` is ONE JSON column shared by every per-session
    /// extension (`goal.v0`, `run_state.*`, `todo.v0`, `workspace_skills.v1`).
    /// The pre-existing writer pattern — `get_session` → mutate the whole
    /// `ExtensionData` → `update().extension_data(..)` — reads and writes in
    /// two separate statements, so two overlapping writers each serialize a
    /// stale snapshot of the WHOLE object and the later commit silently erases
    /// the earlier one, even when they touched different keys.
    ///
    /// `update_extension_state` closes that by doing the read-modify-write of a
    /// SINGLE key inside one transaction that opens with a write (the same
    /// load-bearing trick as `replace_conversation_inner`: a deferred WAL
    /// transaction that reads first pins a snapshot and gets an immediate
    /// `SQLITE_BUSY_SNAPSHOT` on upgrade, bypassing the busy timeout).
    mod extension_state_atomicity {
        use super::*;

        const KEY: &str = "workspace_skills";
        const VER: &str = "v1";

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn concurrent_updates_of_one_key_never_lose_a_write() {
            let store = TempDir::new().unwrap();
            let sm = Arc::new(SessionManager::new(store.path().to_path_buf()));
            let session = sm
                .create_session(store.path().into(), "s".into(), SessionType::User)
                .await
                .unwrap();

            let mut tasks = Vec::new();
            for i in 0..8 {
                let sm = Arc::clone(&sm);
                let id = session.id.clone();
                tasks.push(tokio::spawn(async move {
                    sm.update_extension_state(&id, KEY, VER, move |current| {
                        let mut names: Vec<String> = current
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        names.push(format!("skill-{i}"));
                        Ok(serde_json::to_value(names)?)
                    })
                    .await
                    .unwrap()
                }));
            }
            for task in tasks {
                task.await.unwrap();
            }

            let stored = sm
                .get_extension_state(&session.id, KEY, VER)
                .await
                .unwrap()
                .expect("state persisted");
            let names: Vec<String> = serde_json::from_value(stored).unwrap();
            assert_eq!(
                names.len(),
                8,
                "every concurrent append must survive; got {names:?}"
            );
        }

        /// The blast radius of the stale-snapshot write: a skill grant must not
        /// erase a goal, a todo list, or a paused approval that a concurrent
        /// turn wrote to a DIFFERENT key of the same column.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_concurrent_write_to_another_key_is_not_erased() {
            let store = TempDir::new().unwrap();
            let sm = Arc::new(SessionManager::new(store.path().to_path_buf()));
            let session = sm
                .create_session(store.path().into(), "s".into(), SessionType::User)
                .await
                .unwrap();

            let skills = {
                let sm = Arc::clone(&sm);
                let id = session.id.clone();
                tokio::spawn(async move {
                    sm.update_extension_state(&id, KEY, VER, |_| {
                        Ok(serde_json::json!({"add": ["single-cell"]}))
                    })
                    .await
                })
            };
            let goal = {
                let sm = Arc::clone(&sm);
                let id = session.id.clone();
                tokio::spawn(async move {
                    sm.update_extension_state(&id, "goal", "v0", |_| {
                        Ok(serde_json::json!({"text": "finish BR-71"}))
                    })
                    .await
                })
            };
            skills.await.unwrap().unwrap();
            goal.await.unwrap().unwrap();

            let reread = sm.get_session(&session.id, false).await.unwrap();
            assert_eq!(
                reread
                    .extension_data
                    .get_extension_state(KEY, VER)
                    .and_then(|v| v["add"][0].as_str()),
                Some("single-cell"),
            );
            assert_eq!(
                reread
                    .extension_data
                    .get_extension_state("goal", "v0")
                    .and_then(|v| v["text"].as_str()),
                Some("finish BR-71"),
                "a concurrent write to another extension's key must survive"
            );
        }

        /// The persisted value is the basis for the next merge — not a
        /// process-local cache. A reader opened after the writer's process
        /// would have exited sees the committed value.
        #[tokio::test]
        async fn the_merge_basis_is_the_persisted_value_not_an_in_memory_one() {
            let store = TempDir::new().unwrap();
            let id = {
                let sm = SessionManager::new(store.path().to_path_buf());
                let session = sm
                    .create_session(store.path().into(), "s".into(), SessionType::User)
                    .await
                    .unwrap();
                sm.update_extension_state(&session.id, KEY, VER, |current| {
                    assert!(current.is_none(), "nothing persisted yet");
                    Ok(serde_json::json!({"add": ["proteomics"]}))
                })
                .await
                .unwrap();
                sm.close().await;
                session.id
            };

            // A cold manager over the same directory: no shared in-process state.
            let cold = SessionManager::new(store.path().to_path_buf());
            let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
            let sink = std::sync::Arc::clone(&seen);
            cold.update_extension_state(&id, KEY, VER, move |current| {
                *sink.lock().unwrap() = current.cloned();
                Ok(serde_json::json!({"add": ["proteomics", "single-cell"]}))
            })
            .await
            .unwrap();
            assert_eq!(
                seen.lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|v| v["add"][0].as_str()),
                Some("proteomics"),
                "the mutator must see the persisted value, not an empty default"
            );
        }

        #[tokio::test]
        async fn a_missing_session_reports_not_found_instead_of_creating_one() {
            let store = TempDir::new().unwrap();
            let sm = SessionManager::new(store.path().to_path_buf());
            let outcome = sm
                .update_extension_state("no-such-session", KEY, VER, |_| {
                    Ok(serde_json::json!({"add": []}))
                })
                .await
                .unwrap();
            assert!(outcome.is_none(), "no row to update");
            assert!(sm
                .get_extension_state("no-such-session", KEY, VER)
                .await
                .unwrap()
                .is_none());
        }
    }

    #[tokio::test]
    async fn clear_all_sessions_removes_history_usage_and_side_tables() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let first = sm
            .create_session(temp_dir.path().into(), "First".into(), SessionType::User)
            .await
            .unwrap();
        sm.create_session(temp_dir.path().into(), "Hidden".into(), SessionType::Hidden)
            .await
            .unwrap();
        sm.record_token_event(
            &first.id,
            Some(100),
            Some(20),
            120,
            Some("model"),
            Some("provider"),
            Some(0),
            Some(0),
        )
        .await
        .unwrap();

        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO checkpoints (id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha) VALUES ('cp', ?, 0, 1, 'pre_step', 'commit', 'tree')",
        )
        .bind(&first.id)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO message_blobs (blob_uid, session_id, created_at, bytes, content) VALUES ('blob', ?, 1, 4, 'data')",
        )
        .bind(&first.id)
        .execute(pool)
        .await
        .unwrap();
        // A chat deleted earlier leaves its spend in `deleted_chat_usage` (F10);
        // a reset takes that too.
        let deleted = sm
            .create_session(temp_dir.path().into(), "Deleted".into(), SessionType::User)
            .await
            .unwrap();
        assert!(sm
            .apply_usage_event(billed_turn(&deleted.id, "deleted-1", "p", "m", 30))
            .await
            .unwrap());
        sm.delete_session(&deleted.id).await.unwrap();

        assert_eq!(sm.count_all_sessions().await.unwrap(), 2);
        assert_eq!(sm.clear_all_sessions().await.unwrap(), 2);
        assert_eq!(sm.count_all_sessions().await.unwrap(), 0);
        for table in [
            "sessions",
            "messages",
            "messages_fts",
            "token_events",
            "deleted_chat_usage",
            "checkpoints",
            "message_blobs",
        ] {
            let count = sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(pool)
                .await
                .unwrap();
            assert_eq!(count, 0, "{table} should be empty");
        }
        assert_eq!(sm.get_usage_summary().await.unwrap().all_time.turns, 0);
    }

    /// One billed provider call, recorded through the production writer
    /// (`apply_usage_event`), so a fixture row carries exactly the columns a
    /// real turn writes.
    fn billed_turn(
        session_id: &str,
        event_key: &str,
        provider: &str,
        model: &str,
        billed: i32,
    ) -> UsageLedgerEntry {
        UsageLedgerEntry {
            event_key: event_key.to_string(),
            session_id: session_id.to_string(),
            schedule_id: None,
            current_total_tokens: Some(billed),
            current_input_tokens: Some(billed),
            current_output_tokens: Some(0),
            billed_total_tokens: Some(i64::from(billed)),
            input_tokens: Some(billed),
            output_tokens: Some(0),
            model_id: Some(model.to_string()),
            provider: Some(provider.to_string()),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
        }
    }

    async fn token_event_count(sm: &SessionManager, session_id: &str) -> i64 {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_scalar("SELECT COUNT(*) FROM token_events WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// F10 of the 2026-09-10 provider QA run (M24 of the 2026-09-09 drive).
    /// Deleting a chat removed its messages, blobs and row, and left every
    /// `token_events` row it had written — `model_id`, `provider` and a
    /// timestamp per turn, a record of which model answered when for a chat the
    /// user chose to delete.
    ///
    /// The rows now go with the chat; what they cost does not. The Usage panel
    /// is held against the provider's own meter (issue #1), so the spend is
    /// folded into `deleted_chat_usage` first — per day, model and provider,
    /// with nothing that names the chat — while Home, which counts activity,
    /// lets the chat go entirely.
    #[tokio::test]
    async fn deleting_a_chat_keeps_its_spend_anonymously_and_deletes_its_rows() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let mut chats = Vec::new();
        for name in ["Deleted", "Deleted too", "Kept"] {
            let chat = sm
                .create_session(temp_dir.path().into(), name.into(), SessionType::User)
                .await
                .unwrap();
            chats.push(chat.id);
        }
        let [deleted, deleted_too, kept] = [&chats[0], &chats[1], &chats[2]];
        for entry in [
            billed_turn(deleted, "deleted-1", "p", "m", 100),
            billed_turn(deleted, "deleted-2", "p", "m", 200),
            billed_turn(deleted_too, "deleted-too-1", "p", "m", 25),
            billed_turn(kept, "kept-1", "p", "m", 50),
        ] {
            assert!(sm.apply_usage_event(entry).await.unwrap());
        }
        let spend_before = sm.get_usage_summary().await.unwrap().all_time;
        assert_eq!(
            (spend_before.turns, spend_before.total_tokens),
            (4, Some(375))
        );

        sm.delete_session(deleted).await.unwrap();
        sm.delete_session(deleted_too).await.unwrap();

        // The per-turn trail is gone — and only the deleted chats'.
        assert_eq!(
            token_event_count(&sm, deleted).await,
            0,
            "a deleted chat's usage rows must be deleted with it"
        );
        assert_eq!(token_event_count(&sm, deleted_too).await, 0);
        assert_eq!(
            token_event_count(&sm, kept).await,
            1,
            "the delete is scoped to the chat being deleted"
        );

        // The spend is not refunded, however the Usage panel asks...
        let spend_after = sm.get_usage_summary().await.unwrap();
        assert_eq!(
            spend_after.all_time, spend_before,
            "a delete does not refund spend"
        );
        assert_eq!(spend_after.month_to_date.total_tokens, Some(375));
        let pool = sm.storage().pool().await.unwrap();
        // ...including through the panel's own window: local midnight on the
        // 1st, through now (`UsageSection.tsx`).
        let month_start: i64 = sqlx::query_scalar(
            "SELECT CAST(strftime('%s', strftime('%Y-%m-01 00:00:00', 'now', 'localtime'), 'utc') AS INTEGER)",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let now = chrono::Utc::now().timestamp();
        let days = sm
            .get_usage_report(month_start, now, UsageGroup::Day)
            .await
            .unwrap();
        let report_tokens: i64 = days.iter().filter_map(|row| row.total_tokens).sum();
        assert_eq!(report_tokens, 375, "the Usage panel's day bars: {days:?}");

        // ...while Home counts only the chat that is left.
        assert_eq!(sm.get_insights().await.unwrap().total_tokens, Some(50));

        // Two deleted chats on one day and model share one bucket, and nothing
        // in it can name either of them.
        let buckets: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
            "SELECT model_id, provider, turns, billed_total_tokens, billed_known \
             FROM deleted_chat_usage",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(buckets, vec![("m".into(), "p".into(), 3, 325, 3)]);
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('deleted_chat_usage')")
                .fetch_all(pool)
                .await
                .unwrap();
        for identifying in ["session_id", "event_key", "ts"] {
            assert!(
                !columns.iter().any(|column| column == identifying),
                "deleted_chat_usage must not carry `{identifying}`: {columns:?}"
            );
        }
    }

    /// Why F10 is more than hygiene. `create_session` mints `<day>_<MAX(N)+1>`,
    /// so deleting the newest chat of the day hands its id to the next chat
    /// created that day — "delete the chat I just made and start again" — and
    /// before the fix the new chat inherited the old one's ledger: its cost
    /// popover (`get_session_model_usage`) listed turns it never ran, on a
    /// provider it may never have used. Joining `sessions` cannot catch this,
    /// because the id exists again; only deleting the rows with the chat does.
    #[tokio::test]
    async fn a_reissued_session_id_does_not_inherit_the_deleted_chats_usage() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let first = sm
            .create_session(temp_dir.path().into(), "First".into(), SessionType::User)
            .await
            .unwrap();
        assert!(sm
            .apply_usage_event(billed_turn(
                &first.id,
                "first-1",
                "versa_azure",
                "gpt-5.5",
                900
            ))
            .await
            .unwrap());
        sm.delete_session(&first.id).await.unwrap();

        let second = sm
            .create_session(temp_dir.path().into(), "Second".into(), SessionType::User)
            .await
            .unwrap();
        assert_eq!(
            second.id, first.id,
            "the fixture must reproduce the id reuse, or this test proves nothing"
        );
        let rows = sm.get_session_model_usage(&second.id).await.unwrap();
        assert!(
            rows.is_empty(),
            "a new chat inherited the deleted chat's turns: {rows:?}"
        );
    }

    /// The read-side half of F10. A per-turn row whose chat no longer exists —
    /// left by a build without the delete fix that shares this database, until
    /// the next open folds it — must never be read, whichever surface asks. Its
    /// spend reaches the Usage totals only through the anonymous fold, never
    /// through the row that names the chat. Each aggregation is asked
    /// separately because each is its own statement.
    #[tokio::test]
    async fn usage_aggregations_never_count_a_token_event_whose_chat_is_gone() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let live = seed_session_with_messages(&sm, 1).await;
        assert!(sm
            .apply_usage_event(billed_turn(&live.id, "live-1", "p", "m", 50))
            .await
            .unwrap());

        // An orphan exactly as the pre-fix delete left one: a complete, billable
        // row (its `session_type` captured as 'user' when the turn was written)
        // whose chat is gone. Written raw because no current path produces one.
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens, \
             billed_total_tokens, model_id, provider, cache_read_tokens, cache_creation_tokens, \
             event_key, session_type) \
             VALUES ('deleted_chat', CAST(strftime('%s', 'now') AS INTEGER), 7000, 0, 7000, 7000, \
                     'orphan-model', 'orphan-provider', 0, 0, 'orphan-1', 'user')",
        )
        .execute(pool)
        .await
        .unwrap();

        let insights = sm.get_insights().await.unwrap();
        assert_eq!(insights.total_tokens, Some(50), "insights: all time");
        assert_eq!(insights.tokens_last_7_days, Some(50), "insights: 7 days");
        assert_eq!(insights.tokens_last_30_days, Some(50), "insights: 30 days");

        let activity = sm.get_activity(30).await.unwrap();
        let heatmap_tokens: i64 = activity.days.iter().map(|day| day.tokens).sum();
        assert_eq!(heatmap_tokens, 50, "Home heatmap");

        let summary = sm.get_usage_summary().await.unwrap();
        assert_eq!(summary.all_time.total_tokens, Some(50), "summary: all time");
        assert_eq!(summary.all_time.turns, 1, "summary: all-time turns");
        assert_eq!(
            summary.month_to_date.total_tokens,
            Some(50),
            "summary: month to date"
        );

        let models = sm.get_model_usage(0, i64::MAX).await.unwrap();
        assert!(
            models
                .iter()
                .all(|row| row.model_id.as_deref() != Some("orphan-model")),
            "per-model usage counted the orphan: {models:?}"
        );

        let report = sm
            .get_usage_report(0, i64::MAX, UsageGroup::Model)
            .await
            .unwrap();
        let report_tokens: i64 = report.iter().filter_map(|row| row.total_tokens).sum();
        assert_eq!(report_tokens, 50, "usage report: {report:?}");
    }

    /// F10's existing orphans are retired — folded, then deleted — by the
    /// reconcile that runs every time a store is opened, not by a numbered
    /// migration arm, for the reason `prune_orphaned_token_events` gives. Pinned
    /// here: a billable orphan's spend moves into the anonymous total, so the
    /// Usage numbers an upgrade finds are the numbers it leaves; an unclassified
    /// orphan, which no total ever counted, just goes; a live chat's rows are
    /// never touched, however old; and a second open changes nothing, so no
    /// spend is ever folded twice.
    #[tokio::test]
    async fn reopening_the_store_retires_token_events_whose_chat_is_gone() {
        let temp_dir = TempDir::new().unwrap();
        let live_id = {
            let sm = SessionManager::new(temp_dir.path().to_path_buf());
            let live = sm
                .create_session(temp_dir.path().into(), "Live".into(), SessionType::User)
                .await
                .unwrap();
            assert!(sm
                .apply_usage_event(billed_turn(&live.id, "live-1", "p", "m", 50))
                .await
                .unwrap());
            let pool = sm.storage().pool().await.unwrap();
            for statement in [
                // Two modern orphans, as the pre-fix delete left them.
                "INSERT INTO token_events (session_id, ts, total_tokens, billed_total_tokens, \
                 model_id, provider, event_key, session_type) \
                 VALUES ('gone_1', 100, 10, 10, 'm', 'p', 'gone-1-a', 'user')",
                "INSERT INTO token_events (session_id, ts, total_tokens, billed_total_tokens, \
                 model_id, provider, event_key, session_type) \
                 VALUES ('gone_1', 200, 10, 10, 'm', 'p', 'gone-1-b', 'user')",
                // A legacy orphan: no identity and no captured type.
                "INSERT INTO token_events (session_id, ts, total_tokens) VALUES ('gone_2', 300, 10)",
            ] {
                sqlx::query(statement).execute(pool).await.unwrap();
            }
            // A row older than its own chat, whose chat still exists. The sweep
            // keys on existence alone, so it stays — and the migration fixtures
            // in this module, which date their rows at `ts = 100`, rely on that.
            sqlx::query(
                "INSERT INTO token_events (session_id, ts, total_tokens, billed_total_tokens, \
                 session_type) VALUES (?, 100, 5, 5, 'user')",
            )
            .bind(&live.id)
            .execute(pool)
            .await
            .unwrap();
            sm.close().await;
            live.id
        };

        for open in 1..=2 {
            let sm = SessionManager::new(temp_dir.path().to_path_buf());
            assert_eq!(token_event_count(&sm, "gone_1").await, 0, "open {open}");
            assert_eq!(token_event_count(&sm, "gone_2").await, 0, "open {open}");
            assert_eq!(
                token_event_count(&sm, &live_id).await,
                2,
                "open {open}: a live chat's rows are never swept"
            );
            // The live chat's 50 + 5, plus `gone_1`'s 10 + 10 folded; `gone_2`
            // was never billable. Exactly what the unjoined queries counted.
            let spend = sm.get_usage_summary().await.unwrap().all_time;
            assert_eq!(
                (spend.turns, spend.total_tokens),
                (4, Some(75)),
                "open {open}: the Usage totals an upgrade finds are the ones it leaves"
            );
            assert_eq!(
                sm.get_insights().await.unwrap().total_tokens,
                Some(55),
                "open {open}: Home counts the live chat only"
            );
            sm.close().await;
        }
    }

    #[tokio::test]
    async fn fresh_database_contains_full_v16_schema() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let pool = sm.storage().pool().await.unwrap();

        assert_eq!(
            SessionStorage::get_schema_version(pool).await.unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        for table in [
            "token_events",
            "deleted_chat_usage",
            "checkpoints",
            "messages_fts",
            "message_blobs",
        ] {
            let exists: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?1")
                    .bind(table)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            assert_eq!(exists, 1, "missing fresh-schema table {table}");
        }
        for column in [
            "model_id",
            "provider",
            "cache_read_tokens",
            "cache_creation_tokens",
            "billed_total_tokens",
            "event_key",
            "session_type",
        ] {
            assert!(
                SessionStorage::table_has_column(pool, "token_events", column)
                    .await
                    .unwrap(),
                "missing fresh usage column {column}"
            );
        }
        assert!(
            SessionStorage::table_has_column(pool, "messages", "msg_uid")
                .await
                .unwrap()
        );
        assert!(
            SessionStorage::table_has_column(pool, "sessions", "branch_point_msg_uid")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn checkpoints_table_crud_roundtrip() {
        // A fresh DB (create_schema path) must carry the migration-13 `checkpoints`
        // table, and the CRUD helpers `CheckpointManager` relies on must roundtrip.
        use crate::checkpoint::{CheckpointKind, CheckpointRecord};
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        assert!(sm.list_checkpoints("s1").await.unwrap().is_empty());
        assert!(sm.last_checkpoint("s1").await.unwrap().is_none());

        let rec = CheckpointRecord {
            id: "cp-1".to_string(),
            session_id: "s1".to_string(),
            turn_index: 0,
            anchor_ts: 1234,
            kind: CheckpointKind::PreStep,
            commit_sha: "deadbeef".to_string(),
            tree_sha: "cafef00d".to_string(),
            changed_paths: vec!["a.txt".to_string()],
            created_at: "2026-07-12T00:00:00Z".to_string(),
        };
        sm.insert_checkpoint(&rec).await.unwrap();

        let listed = sm.list_checkpoints("s1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, CheckpointKind::PreStep);
        assert_eq!(listed[0].tree_sha, "cafef00d");
        assert_eq!(listed[0].changed_paths, vec!["a.txt".to_string()]);

        let got = sm.get_checkpoint("s1", "cp-1").await.unwrap().unwrap();
        assert_eq!(got.commit_sha, "deadbeef");
        assert!(sm.get_checkpoint("s1", "missing").await.unwrap().is_none());
        // Scoped by session.
        assert!(sm.get_checkpoint("other", "cp-1").await.unwrap().is_none());

        sm.delete_checkpoints("s1").await.unwrap();
        assert!(sm.list_checkpoints("s1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_concurrent_session_creation() {
        let temp_dir = TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));

        let mut handles = vec![];

        for i in 0..NUM_CONCURRENT_SESSIONS {
            let sm = Arc::clone(&session_manager);
            let handle = tokio::spawn(async move {
                let working_dir = PathBuf::from(format!("/tmp/test_{}", i));
                let description = format!("Test session {}", i);

                let session = sm
                    .create_session(working_dir.clone(), description, SessionType::User)
                    .await
                    .unwrap();

                sm.add_message(
                    &session.id,
                    &Message {
                        id: None,
                        role: Role::User,
                        created: chrono::Utc::now().timestamp_millis(),
                        content: vec![MessageContent::text("hello world")],
                        metadata: Default::default(),
                    },
                )
                .await
                .unwrap();

                sm.add_message(
                    &session.id,
                    &Message {
                        id: None,
                        role: Role::Assistant,
                        created: chrono::Utc::now().timestamp_millis(),
                        content: vec![MessageContent::text("sup world?")],
                        metadata: Default::default(),
                    },
                )
                .await
                .unwrap();

                sm.update(&session.id)
                    .user_provided_name(format!("Updated session {}", i))
                    .total_tokens(Some(100 * i))
                    .apply()
                    .await
                    .unwrap();
                sm.record_token_event(
                    &session.id,
                    Some(100 * i),
                    Some(0),
                    i64::from(100 * i),
                    Some("test-model"),
                    Some("test-provider"),
                    Some(0),
                    Some(0),
                )
                .await
                .unwrap();

                let updated = sm.get_session(&session.id, true).await.unwrap();
                assert_eq!(updated.message_count, 2);
                assert_eq!(updated.total_tokens, Some(100 * i));

                session.id
            });
            handles.push(handle);
        }

        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        assert_eq!(results.len(), NUM_CONCURRENT_SESSIONS as usize);

        let unique_ids: std::collections::HashSet<_> = results.iter().collect();
        assert_eq!(unique_ids.len(), NUM_CONCURRENT_SESSIONS as usize);

        let sessions = session_manager.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), NUM_CONCURRENT_SESSIONS as usize);

        for session in &sessions {
            assert_eq!(session.message_count, 2);
            assert!(session.name.starts_with("Updated session"));
        }

        let insights = session_manager.get_insights().await.unwrap();
        assert_eq!(insights.total_sessions, NUM_CONCURRENT_SESSIONS as usize);
        let expected_tokens = 100 * NUM_CONCURRENT_SESSIONS * (NUM_CONCURRENT_SESSIONS - 1) / 2;
        assert_eq!(insights.total_tokens, Some(expected_tokens as i64));
    }

    #[tokio::test]
    async fn test_export_import_roundtrip() {
        const DESCRIPTION: &str = "Original session";
        const TOTAL_TOKENS: i32 = 500;
        const INPUT_TOKENS: i32 = 300;
        const OUTPUT_TOKENS: i32 = 200;
        const ACCUMULATED_TOKENS: i64 = 1000;
        const USER_MESSAGE: &str = "test message";
        const ASSISTANT_MESSAGE: &str = "test response";

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                DESCRIPTION.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        sm.update(&original.id)
            .total_tokens(Some(TOTAL_TOKENS))
            .input_tokens(Some(INPUT_TOKENS))
            .output_tokens(Some(OUTPUT_TOKENS))
            .accumulated_total_tokens(Some(ACCUMULATED_TOKENS))
            .apply()
            .await
            .unwrap();

        sm.add_message(
            &original.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text(USER_MESSAGE)],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        sm.add_message(
            &original.id,
            &Message {
                id: None,
                role: Role::Assistant,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text(ASSISTANT_MESSAGE)],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let exported = sm.export_session(&original.id).await.unwrap();
        let imported = sm.import_session(&exported).await.unwrap();

        assert_ne!(imported.id, original.id);
        assert_eq!(imported.name, DESCRIPTION);
        assert_eq!(imported.working_dir, PathBuf::from("/tmp/test"));
        assert_eq!(imported.total_tokens, Some(TOTAL_TOKENS));
        assert_eq!(imported.input_tokens, Some(INPUT_TOKENS));
        assert_eq!(imported.output_tokens, Some(OUTPUT_TOKENS));
        assert_eq!(imported.accumulated_total_tokens, Some(ACCUMULATED_TOKENS));
        assert_eq!(imported.message_count, 2);

        let conversation = imported.conversation.unwrap();
        assert_eq!(conversation.messages().len(), 2);
        assert_eq!(conversation.messages()[0].role, Role::User);
        assert_eq!(conversation.messages()[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn durable_session_resumes_by_external_key() {
        // BRSDK durable sessions: the same external key resumes the same
        // session (with its conversation), distinct keys stay isolated, and a
        // reconnect recovers prior messages — the "recover what it lost" path.
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let key1 = "app:demo:client-1";
        let (s1, resumed1) = sm
            .get_or_create_by_external_key(
                key1,
                PathBuf::from("/tmp/app"),
                "app:demo".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(!resumed1, "first call creates, does not resume");

        // Simulate a turn of conversation on this session.
        sm.add_message(
            &s1.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("what is CFTR?")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        // Reconnect with the SAME external key → resume the SAME session.
        let (s2, resumed2) = sm
            .get_or_create_by_external_key(
                key1,
                PathBuf::from("/tmp/app"),
                "app:demo".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(resumed2, "same key must resume");
        assert_eq!(s2.id, s1.id, "resumed session keeps its id");
        assert_eq!(s2.message_count, 1, "prior conversation is recovered");
        let convo = s2
            .conversation
            .expect("resumed session carries conversation");
        assert_eq!(convo.messages().len(), 1);
        assert_eq!(convo.messages()[0].role, Role::User);

        // A different client key → a separate, isolated session.
        let (s3, resumed3) = sm
            .get_or_create_by_external_key(
                "app:demo:client-2",
                PathBuf::from("/tmp/app"),
                "app:demo".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(!resumed3);
        assert_ne!(s3.id, s1.id, "distinct keys are isolated");
        assert_eq!(s3.message_count, 0);

        // A different app entirely → also isolated.
        let (s4, _) = sm
            .get_or_create_by_external_key(
                "app:other:client-1",
                PathBuf::from("/tmp/app"),
                "app:other".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert_ne!(s4.id, s1.id);
    }

    #[tokio::test]
    async fn durable_session_survives_a_fresh_manager_on_same_dir() {
        // The external_key binding is persisted, so a brand-new SessionManager
        // over the same data dir (e.g. a daemon restart) still resumes it.
        let temp_dir = TempDir::new().unwrap();
        let key = "app:persist:client-x";

        let id_first = {
            let sm = SessionManager::new(temp_dir.path().to_path_buf());
            let (s, resumed) = sm
                .get_or_create_by_external_key(
                    key,
                    PathBuf::from("/tmp/app"),
                    "app:persist".to_string(),
                    SessionType::User,
                )
                .await
                .unwrap();
            assert!(!resumed);
            s.id
        };

        // New manager instance, same on-disk DB.
        let sm2 = SessionManager::new(temp_dir.path().to_path_buf());
        let (s2, resumed2) = sm2
            .get_or_create_by_external_key(
                key,
                PathBuf::from("/tmp/app"),
                "app:persist".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(resumed2, "binding persists across manager instances");
        assert_eq!(s2.id, id_first);
    }

    #[tokio::test]
    async fn migrates_v7_db_to_v8_external_key() {
        // The production upgrade path: every existing user has a v7 DB. Hand-roll
        // a v7-shaped DB, then open the real manager and confirm it migrates to
        // v8 (adds external_key + the partial unique index) WITHOUT losing data.
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();

            sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)")
                .execute(&pool).await.unwrap();
            for v in 1..=7 {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(v)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            // The v7 sessions table = current schema MINUS external_key.
            sqlx::query(
                r#"CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
                    user_set_name BOOLEAN DEFAULT FALSE, session_type TEXT NOT NULL DEFAULT 'user', working_dir TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    extension_data TEXT DEFAULT '{}', total_tokens INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                    accumulated_total_tokens INTEGER, accumulated_input_tokens INTEGER, accumulated_output_tokens INTEGER,
                    schedule_id TEXT, workflow_json TEXT, user_workflow_values_json TEXT, provider_name TEXT, model_config_json TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query(
                r#"CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
                    role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL,
                    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, tokens INTEGER, metadata_json TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            // A pre-existing (legacy) session that must survive the migration.
            sqlx::query("INSERT INTO sessions (id, name, working_dir) VALUES ('20240101_1', 'legacy session', '/tmp/old')")
                .execute(&pool).await.unwrap();
            pool.close().await;
        }

        // Opening the real manager triggers run_migrations → the `8 =>` arm.
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // Legacy data survived the ALTER TABLE.
        let legacy = sm.get_session("20240101_1", true).await.unwrap();
        assert_eq!(legacy.name, "legacy session");
        assert_eq!(legacy.working_dir, PathBuf::from("/tmp/old"));

        // Messages table still works post-migration.
        sm.add_message(
            "20240101_1",
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("post-migration message")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            sm.get_session("20240101_1", true)
                .await
                .unwrap()
                .message_count,
            1
        );

        // The migrated external_key column + unique index are queryable: a
        // second call with the same key resumes (proves the index exists).
        let (s1, r1) = sm
            .get_or_create_by_external_key(
                "app:x:c1",
                PathBuf::from("/tmp/app"),
                "app:x".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(!r1);
        let (s2, r2) = sm
            .get_or_create_by_external_key(
                "app:x:c1",
                PathBuf::from("/tmp/app"),
                "app:x".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(r2, "external_key index works after migration");
        assert_eq!(s1.id, s2.id);

        // Two NULL external_keys are allowed (partial unique index) — the legacy
        // row + a fresh plain session both have NULL and coexist.
        let plain = sm
            .create_session(
                PathBuf::from("/tmp"),
                "plain".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert_ne!(plain.id, "20240101_1");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_same_key_callers_converge_on_one_session() {
        // The race-safe claim: many truly-concurrent callers with the SAME key
        // must all resolve to exactly ONE session (no duplicates, no errors).
        let temp_dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let key = "app:race:client-1";

        let mut handles = Vec::new();
        for _ in 0..8 {
            let sm = sm.clone();
            let key = key.to_string();
            handles.push(tokio::spawn(async move {
                sm.get_or_create_by_external_key(
                    &key,
                    PathBuf::from("/tmp/app"),
                    "app:race".to_string(),
                    SessionType::User,
                )
                .await
            }));
        }

        let mut ids = std::collections::HashSet::new();
        for h in handles {
            let (s, _resumed) = h.await.unwrap().expect("no caller should error");
            ids.insert(s.id);
        }
        assert_eq!(
            ids.len(),
            1,
            "all concurrent same-key callers must converge on exactly one session"
        );
    }

    #[tokio::test]
    async fn test_import_session_with_description_field() {
        const OLD_FORMAT_JSON: &str = r#"{
            "id": "20240101_1",
            "description": "Old format session",
            "user_set_name": true,
            "working_dir": "/tmp/test",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "extension_data": {},
            "message_count": 0
        }"#;

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let imported = sm.import_session(OLD_FORMAT_JSON).await.unwrap();

        assert_eq!(imported.name, "Old format session");
        assert!(imported.user_set_name);
        assert_eq!(imported.working_dir, PathBuf::from("/tmp/test"));
    }

    // ── Diverge (copy_session) tests ────────────────────────────────────────
    //
    // `copy_session` is the engine behind both the edit-diverge path and the
    // `/diverge` feature (Diverge button + `/diverge` slash command). Diverge
    // copies the *entire* conversation with no truncation, so the new session
    // resumes from exactly where the original left off while the original stays
    // put. These tests exercise that contract from many angles.

    /// Seed a User session with `n` user/assistant message pairs and return it
    /// (loaded with its conversation).
    async fn seed_session_with_messages(sm: &SessionManager, n: usize) -> Session {
        let session = sm
            .create_session(
                PathBuf::from("/tmp/diverge_test"),
                "Original".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        for i in 0..n {
            sm.add_message(
                &session.id,
                &Message {
                    id: None,
                    role: Role::User,
                    created: chrono::Utc::now().timestamp_millis() + (i as i64) * 2,
                    content: vec![MessageContent::text(format!("question {i}"))],
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap();
            sm.add_message(
                &session.id,
                &Message {
                    id: None,
                    role: Role::Assistant,
                    created: chrono::Utc::now().timestamp_millis() + (i as i64) * 2 + 1,
                    content: vec![MessageContent::text(format!("answer {i}"))],
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap();
        }

        sm.get_session(&session.id, true).await.unwrap()
    }

    #[tokio::test]
    async fn session_summaries_are_lightweight_and_paginated() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let created = vec![
            seed_session_with_messages(&sm, 1).await,
            seed_session_with_messages(&sm, 2).await,
            seed_session_with_messages(&sm, 3).await,
        ];

        let first_page = sm.list_session_summaries(2, 0, false, false).await.unwrap();
        let second_page = sm.list_session_summaries(2, 2, false, false).await.unwrap();

        assert_eq!(first_page.len(), 2);
        assert_eq!(second_page.len(), 1);
        assert!(first_page.iter().all(|session| session.message_count > 0));
        assert!(first_page
            .iter()
            .all(|session| session.working_dir == "/tmp/diverge_test"));

        let expected_ids: std::collections::HashSet<_> =
            created.into_iter().map(|session| session.id).collect();
        let actual_ids: std::collections::HashSet<_> = first_page
            .into_iter()
            .chain(second_page)
            .map(|session| session.id)
            .collect();
        assert_eq!(actual_ids, expected_ids);
    }

    #[tokio::test]
    async fn list_session_summaries_hides_subagents_unless_asked() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let parent = manager
            .create_session(
                temp.path().to_path_buf(),
                "p".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let child = manager
            .create_session(
                temp.path().to_path_buf(),
                "c".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        manager
            .update(&child.id)
            .parent_session_id(Some(parent.id.clone()))
            .apply()
            .await
            .unwrap();
        // `include_subagents` widens the type filter by exactly one type. A
        // regression that dropped the `WHERE s.session_type IN (…)` clause
        // altogether would satisfy every assertion about `child` while quietly
        // leaking `hidden` sessions, which this codebase excludes from user-
        // facing listings everywhere else (see
        // `usage_report_includes_subagent_spend_and_excludes_hidden_sessions`).
        // This row is the sentinel for that.
        let hidden = manager
            .create_session(
                temp.path().to_path_buf(),
                "h".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();

        // `list_session_summaries` INNER JOINs `messages` and `create_session`
        // writes NO message, so a freshly created session is invisible to this
        // query until it has one. The pre-existing paging test uses
        // `seed_session_with_messages` for exactly this reason. Without these
        // two writes both assertions below fail against an empty row set.
        for s in [&parent, &child, &hidden] {
            manager
                .add_message(
                    &s.id,
                    &crate::conversation::message::Message::user().with_text("x"),
                )
                .await
                .unwrap();
        }

        // (limit, offset, include_subagents, include_empty) — see Step 3.
        let default_list = manager
            .list_session_summaries(50, 0, false, false)
            .await
            .unwrap();
        assert!(default_list.iter().any(|s| s.id == parent.id));
        assert!(!default_list.iter().any(|s| s.id == child.id));
        assert!(!default_list.iter().any(|s| s.id == hidden.id));

        let full = manager
            .list_session_summaries(50, 0, true, false)
            .await
            .unwrap();
        let child_row = full
            .iter()
            .find(|s| s.id == child.id)
            .expect("child listed");
        assert_eq!(
            child_row.parent_session_id.as_deref(),
            Some(parent.id.as_str())
        );
        assert_eq!(child_row.session_type.as_deref(), Some("sub_agent"));
        assert!(!full.iter().any(|s| s.id == hidden.id));
    }

    #[tokio::test]
    async fn test_diverge_preserves_full_history_and_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 3).await;
        assert_eq!(original.message_count, 6);

        let diverged = sm
            .copy_session(&original.id, "Branch".to_string())
            .await
            .unwrap();

        // New, distinct id.
        assert_ne!(diverged.id, original.id);
        // Name applied.
        assert_eq!(diverged.name, "Branch");
        // Working dir carried over.
        assert_eq!(diverged.working_dir, original.working_dir);
        // Full conversation copied verbatim, in order.
        assert_eq!(diverged.message_count, 6);
        let orig_texts: Vec<_> = original
            .conversation
            .as_ref()
            .unwrap()
            .messages()
            .iter()
            .map(|m| m.as_concat_text())
            .collect();
        let new_texts: Vec<_> = diverged
            .conversation
            .as_ref()
            .unwrap()
            .messages()
            .iter()
            .map(|m| m.as_concat_text())
            .collect();
        assert_eq!(orig_texts, new_texts);
    }

    #[tokio::test]
    async fn test_diverge_leaves_original_untouched() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 2).await;
        let diverged = sm
            .copy_session(&original.id, "Branch".to_string())
            .await
            .unwrap();

        // Mutate the diverged session by appending a new message.
        sm.add_message(
            &diverged.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis() + 10_000,
                content: vec![MessageContent::text("only in the branch")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        // Original is completely unaffected: same id, same message count.
        let original_after = sm.get_session(&original.id, true).await.unwrap();
        assert_eq!(original_after.message_count, 4);
        let branch_after = sm.get_session(&diverged.id, true).await.unwrap();
        assert_eq!(branch_after.message_count, 5);

        // Both sessions still exist independently.
        let sessions = sm.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_diverge_resets_token_counts() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 1).await;
        sm.update(&original.id)
            .total_tokens(Some(1234))
            .input_tokens(Some(1000))
            .output_tokens(Some(234))
            .apply()
            .await
            .unwrap();

        let diverged = sm
            .copy_session(&original.id, "Branch".to_string())
            .await
            .unwrap();

        // Diverged session starts with fresh token accounting.
        assert_eq!(diverged.total_tokens, None);
        assert_eq!(diverged.input_tokens, None);
        assert_eq!(diverged.output_tokens, None);

        // Original keeps its counts.
        let original_after = sm.get_session(&original.id, false).await.unwrap();
        assert_eq!(original_after.total_tokens, Some(1234));
    }

    /// The old path read the row into Rust, added, and wrote it back. Two turns
    /// racing on the same session silently lost one update.
    #[tokio::test]
    async fn accumulate_tokens_is_additive_and_atomic() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        for _ in 0..3 {
            sm.update(&session.id)
                .accumulate_tokens(TokenDelta {
                    input: Some(100),
                    output: Some(20),
                    total: Some(120),
                })
                .apply()
                .await
                .unwrap();
        }

        let counts = sm.get_token_counts(&session.id).await.unwrap();
        assert_eq!(counts.accumulated_total_tokens, Some(360));
        assert_eq!(counts.accumulated_input_tokens, Some(300));
        assert_eq!(counts.accumulated_output_tokens, Some(60));
    }

    #[tokio::test]
    async fn usage_event_updates_ledger_and_counters_once_in_one_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;
        let entry = UsageLedgerEntry {
            event_key: "provider-call-1".to_string(),
            session_id: session.id.clone(),
            schedule_id: None,
            current_total_tokens: Some(820),
            current_input_tokens: Some(100),
            current_output_tokens: Some(20),
            billed_total_tokens: Some(900),
            input_tokens: Some(100),
            output_tokens: Some(20),
            model_id: Some("claude-sonnet-4-20250514".to_string()),
            provider: Some("anthropic".to_string()),
            cache_read_tokens: Some(700),
            cache_creation_tokens: Some(80),
        };

        assert!(sm.apply_usage_event(entry.clone()).await.unwrap());
        assert!(
            !sm.apply_usage_event(entry).await.unwrap(),
            "retrying the same provider call is a no-op"
        );

        let counts = sm.get_token_counts(&session.id).await.unwrap();
        assert_eq!(
            counts.total_tokens,
            Some(820),
            "live context stays separate"
        );
        assert_eq!(counts.accumulated_total_tokens, Some(900));
        assert_eq!(counts.accumulated_input_tokens, Some(100));
        assert_eq!(counts.accumulated_output_tokens, Some(20));

        let rows = sm.get_session_model_usage(&session.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turns, 1);
        assert_eq!(rows[0].total_tokens, Some(900));
        assert_eq!(rows[0].cache_read_tokens, Some(700));
        assert_eq!(rows[0].cache_creation_tokens, Some(80));
    }

    #[tokio::test]
    async fn total_only_usage_is_retained_but_never_priced_as_zero() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        sm.apply_usage_event(UsageLedgerEntry {
            event_key: "total-only-provider-call".to_string(),
            session_id: session.id.clone(),
            schedule_id: None,
            current_total_tokens: Some(500),
            current_input_tokens: None,
            current_output_tokens: None,
            billed_total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            model_id: Some("glm-5.2".to_string()),
            provider: Some("zai".to_string()),
            cache_read_tokens: None,
            cache_creation_tokens: None,
        })
        .await
        .unwrap();

        let counts = sm.get_token_counts(&session.id).await.unwrap();
        assert_eq!(counts.total_tokens, Some(500));
        assert_eq!(counts.accumulated_total_tokens, None);

        let rows = sm
            .get_usage_report(0, i64::MAX, UsageGroup::Model)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, None);
        assert_eq!(rows[0].cost, None, "unknown cost is null, never $0");
        assert!(rows[0].has_unpriced);
        let insights = sm.get_insights().await.unwrap();
        assert_eq!(insights.total_tokens, None);
        assert_eq!(insights.tokens_last_7_days, None);
        let activity = sm.get_activity(7).await.unwrap();
        assert!(!activity.tokens_complete);
        assert_eq!(activity.days[0].tokens, 0);
        assert!(!activity.days[0].tokens_complete);
    }

    #[tokio::test]
    async fn modern_zero_cache_buckets_persist_as_complete_measurements() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        sm.apply_usage_event(UsageLedgerEntry {
            event_key: "no-cache-provider-call".to_string(),
            session_id: session.id,
            schedule_id: None,
            current_total_tokens: Some(120),
            current_input_tokens: Some(100),
            current_output_tokens: Some(20),
            billed_total_tokens: Some(120),
            input_tokens: Some(100),
            output_tokens: Some(20),
            model_id: Some("glm-5.2".to_string()),
            provider: Some("zai".to_string()),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
        })
        .await
        .unwrap();

        let rows = sm
            .get_usage_report(0, i64::MAX, UsageGroup::Model)
            .await
            .unwrap();
        assert_eq!(rows[0].cache_read_tokens, Some(0));
        assert_eq!(rows[0].cache_creation_tokens, Some(0));
        assert!(!rows[0].has_unpriced);
        assert!(!rows[0].cost_excludes_cache);
        assert!(rows[0].cost.is_some());
    }

    /// SQLite's INTEGER is 64-bit; the Rust side used to be `i32`, which wraps
    /// negative past ~2.1e9 and then *subtracts* from the insights SUM.
    #[tokio::test]
    async fn accumulated_tokens_exceed_i32() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        let beyond_i32 = i64::from(i32::MAX) + 1_000;
        sm.update(&session.id)
            .accumulated_total_tokens(Some(beyond_i32))
            .apply()
            .await
            .unwrap();

        let counts = sm.get_token_counts(&session.id).await.unwrap();
        assert_eq!(counts.accumulated_total_tokens, Some(beyond_i32));

        sm.record_token_event(
            &session.id,
            None,
            None,
            beyond_i32,
            Some("m"),
            Some("p"),
            None,
            None,
        )
        .await
        .unwrap();

        let insights = sm.get_insights().await.unwrap();
        assert_eq!(
            insights.total_tokens,
            Some(beyond_i32),
            "no wrap, no negative sum"
        );
    }

    /// The tiles on Home must agree with the session list printed beneath them.
    #[tokio::test]
    async fn insights_exclude_internal_session_types() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        for (session_type, tokens) in [
            (SessionType::User, 1_000i64),
            (SessionType::Scheduled, 500),
            (SessionType::SubAgent, 9_000),
            (SessionType::Hidden, 9_000),
            (SessionType::Terminal, 9_000),
        ] {
            let s = sm
                .create_session("/tmp".into(), String::new(), session_type)
                .await
                .unwrap();
            sm.record_token_event(&s.id, None, None, tokens, None, None, None, None)
                .await
                .unwrap();
        }

        let insights = sm.get_insights().await.unwrap();
        assert_eq!(insights.total_sessions, 2, "user + scheduled only");
        assert_eq!(insights.total_tokens, Some(1_500));
    }

    /// The per-turn ledger is what makes a real per-day token series possible —
    /// and what makes "tokens in the last 7 days" mean what it says.
    #[tokio::test]
    async fn token_events_drive_the_windowed_totals() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        // No events yet: the lifetime total is non-zero but the window is empty.
        sm.update(&session.id)
            .accumulated_total_tokens(Some(50_000))
            .apply()
            .await
            .unwrap();
        let insights = sm.get_insights().await.unwrap();
        assert_eq!(insights.total_tokens, Some(0));
        assert_eq!(
            insights.tokens_last_7_days,
            Some(0),
            "a lifetime total is not a 7-day total"
        );

        sm.record_token_event(
            &session.id,
            Some(80),
            Some(20),
            100,
            Some("m1"),
            Some("p1"),
            None,
            None,
        )
        .await
        .unwrap();
        let insights = sm.get_insights().await.unwrap();
        assert_eq!(insights.tokens_last_7_days, Some(100));
        assert_eq!(insights.tokens_last_30_days, Some(100));

        let activity = sm.get_activity(30).await.unwrap();
        assert_eq!(activity.days.len(), 1);
        assert_eq!(activity.days[0].tokens, 100);
        assert!(activity.days[0].tokens_complete);
        assert_eq!(activity.days[0].sessions, 1);
        assert!(activity.days[0].level >= 1);
        assert_eq!(activity.current_streak, 1);
    }

    #[tokio::test]
    async fn per_model_usage_sums_across_models_and_unknown() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        // Two turns on model A, one on model B, one with no model (unknown).
        sm.record_token_event(
            &session.id,
            Some(100),
            Some(20),
            120,
            Some("gpt-5"),
            Some("openai"),
            None,
            None,
        )
        .await
        .unwrap();
        sm.record_token_event(
            &session.id,
            Some(200),
            Some(50),
            250,
            Some("gpt-5"),
            Some("openai"),
            None,
            None,
        )
        .await
        .unwrap();
        sm.record_token_event(
            &session.id,
            Some(10),
            Some(5),
            15,
            Some("claude-fable-5"),
            Some("anthropic"),
            None,
            None,
        )
        .await
        .unwrap();
        // Unknown: no model / provider reported.
        sm.record_token_event(&session.id, Some(1), Some(2), 3, None, None, None, None)
            .await
            .unwrap();

        let rows = sm.get_session_model_usage(&session.id).await.unwrap();
        assert_eq!(
            rows.len(),
            3,
            "gpt-5, claude, and unknown are distinct groups"
        );

        let gpt = rows
            .iter()
            .find(|r| r.model_id.as_deref() == Some("gpt-5"))
            .expect("gpt-5 row present");
        // Hand-computed: 100+200 in, 20+50 out, 120+250 total, 2 turns.
        assert_eq!(gpt.provider.as_deref(), Some("openai"));
        assert_eq!(gpt.input_tokens, 300);
        assert_eq!(gpt.output_tokens, 70);
        assert_eq!(gpt.total_tokens, Some(370));
        assert_eq!(gpt.turns, 2);

        let claude = rows
            .iter()
            .find(|r| r.model_id.as_deref() == Some("claude-fable-5"))
            .expect("claude row present");
        assert_eq!(claude.input_tokens, 10);
        assert_eq!(claude.output_tokens, 5);
        assert_eq!(claude.total_tokens, Some(15));
        assert_eq!(claude.turns, 1);

        let unknown = rows
            .iter()
            .find(|r| r.model_id.is_none())
            .expect("unknown row present");
        assert_eq!(unknown.provider, None);
        assert_eq!(unknown.input_tokens, 1);
        assert_eq!(unknown.output_tokens, 2);
        assert_eq!(unknown.total_tokens, Some(3));
        assert_eq!(unknown.turns, 1);

        // Ordered by total_tokens DESC.
        assert_eq!(rows[0].model_id.as_deref(), Some("gpt-5"));
    }

    /// Shared fixture for the pure rollup tests. Prices use the real zai
    /// `glm-5.2` card ($1.40 / 1M input, $4.40 / 1M output); the unknown-model
    /// row is unpriced. Chosen so every dollar figure is an exact hand value.
    fn usage_grain_fixture() -> Vec<UsageGrainRow> {
        vec![
            // Day 10, priced: 1M input → $1.40.
            UsageGrainRow {
                day: "2026-07-10".into(),
                model_id: Some("glm-5.2".into()),
                provider: Some("zai".into()),
                input_tokens: 1_000_000,
                output_tokens: 0,
                total_tokens: Some(1_000_000),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                turns: 5,
                input_complete: 1,
                output_complete: 1,
                cache_read_complete: 1,
                cache_creation_complete: 1,
            },
            // Day 10, unknown model → unpriced.
            UsageGrainRow {
                day: "2026-07-10".into(),
                model_id: None,
                provider: None,
                input_tokens: 500,
                output_tokens: 500,
                total_tokens: Some(1_000),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                turns: 2,
                input_complete: 1,
                output_complete: 1,
                cache_read_complete: 1,
                cache_creation_complete: 1,
            },
            // Day 11, priced: 2M input + 1M output → 2.80 + 4.40 = $7.20.
            UsageGrainRow {
                day: "2026-07-11".into(),
                model_id: Some("glm-5.2".into()),
                provider: Some("zai".into()),
                input_tokens: 2_000_000,
                output_tokens: 1_000_000,
                total_tokens: Some(3_000_000),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                turns: 3,
                input_complete: 1,
                output_complete: 1,
                cache_read_complete: 1,
                cache_creation_complete: 1,
            },
        ]
    }

    fn approx(a: Option<f64>, b: f64) -> bool {
        matches!(a, Some(v) if (v - b).abs() < 1e-9)
    }

    #[test]
    fn rollup_by_day_prices_each_model_before_summing() {
        let rows = rollup_report(&usage_grain_fixture(), UsageGroup::Day);
        assert_eq!(rows.len(), 2);

        // Chronological: day 10 then day 11.
        let d10 = &rows[0];
        assert_eq!(d10.date.as_deref(), Some("2026-07-10"));
        assert_eq!(d10.model_id, None, "day grouping drops the model");
        assert_eq!(d10.input_tokens, 1_000_500);
        assert_eq!(d10.output_tokens, 500);
        assert_eq!(d10.total_tokens, Some(1_001_000));
        assert_eq!(d10.turns, 7);
        // Only the priced glm row contributes dollars; the unknown row flags it.
        assert!(approx(d10.cost, 1.40), "got {:?}", d10.cost);
        assert!(d10.has_unpriced);

        let d11 = &rows[1];
        assert_eq!(d11.date.as_deref(), Some("2026-07-11"));
        assert!(approx(d11.cost, 7.20), "got {:?}", d11.cost);
        assert!(!d11.has_unpriced);
    }

    #[test]
    fn rollup_by_model_sums_days_and_isolates_unknown() {
        let rows = rollup_report(&usage_grain_fixture(), UsageGroup::Model);
        assert_eq!(rows.len(), 2);

        // Heaviest model first.
        let glm = &rows[0];
        assert_eq!(glm.model_id.as_deref(), Some("glm-5.2"));
        assert_eq!(glm.date, None, "model grouping drops the day");
        assert_eq!(glm.input_tokens, 3_000_000);
        assert_eq!(glm.output_tokens, 1_000_000);
        assert_eq!(glm.total_tokens, Some(4_000_000));
        assert_eq!(glm.turns, 8);
        // 1.40 (day 10) + 7.20 (day 11) = 8.60.
        assert!(approx(glm.cost, 8.60), "got {:?}", glm.cost);
        assert!(!glm.has_unpriced);

        let unknown = &rows[1];
        assert_eq!(unknown.model_id, None);
        assert_eq!(unknown.cost, None, "unknown model is null cost, never $0");
        assert!(unknown.has_unpriced);
    }

    #[test]
    fn rollup_by_day_model_keeps_every_bucket() {
        let rows = rollup_report(&usage_grain_fixture(), UsageGroup::DayModel);
        assert_eq!(rows.len(), 3);
        // Sorted day asc, then heaviest model within a day.
        assert_eq!(
            (rows[0].date.as_deref(), rows[0].model_id.as_deref()),
            (Some("2026-07-10"), Some("glm-5.2"))
        );
        assert!(approx(rows[0].cost, 1.40));
        assert_eq!(
            (rows[1].date.as_deref(), rows[1].model_id.as_deref()),
            (Some("2026-07-10"), None)
        );
        assert_eq!(rows[1].cost, None);
        assert_eq!(
            (rows[2].date.as_deref(), rows[2].model_id.as_deref()),
            (Some("2026-07-11"), Some("glm-5.2"))
        );
        assert!(approx(rows[2].cost, 7.20));
    }

    #[test]
    fn totals_from_grain_sum_and_price() {
        let totals = totals_from_grain(&usage_grain_fixture());
        assert_eq!(totals.input_tokens, 3_000_500);
        assert_eq!(totals.output_tokens, 1_000_500);
        assert_eq!(totals.total_tokens, Some(4_001_000));
        assert_eq!(totals.turns, 10);
        assert!(approx(totals.cost, 8.60), "got {:?}", totals.cost);
        assert!(totals.has_unpriced, "the unknown row leaves it partial");
    }

    #[test]
    fn totals_are_fully_null_when_nothing_is_priced() {
        let grain = vec![UsageGrainRow {
            day: "".into(),
            model_id: None,
            provider: None,
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: Some(150),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 1,
            input_complete: 1,
            output_complete: 1,
            cache_read_complete: 1,
            cache_creation_complete: 1,
        }];
        let totals = totals_from_grain(&grain);
        assert_eq!(totals.total_tokens, Some(150));
        assert_eq!(totals.cost, None, "wholly-unpriced span is null, not $0");
        assert!(totals.has_unpriced);
    }

    #[test]
    fn partial_bucket_with_zero_known_subtotal_has_null_cost() {
        let grain = vec![UsageGrainRow {
            day: "".into(),
            model_id: Some("glm-5.2".into()),
            provider: Some("zai".into()),
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            turns: 1,
            input_complete: 0,
            output_complete: 0,
            cache_read_complete: 0,
            cache_creation_complete: 0,
        }];
        let totals = totals_from_grain(&grain);
        assert_eq!(totals.cost, None, "an unknown cost must not render as $0");
        assert!(totals.has_unpriced);
    }

    #[test]
    fn legacy_null_cache_buckets_remain_incomplete_for_models_without_cache_rates() {
        let grain = vec![UsageGrainRow {
            day: "".into(),
            model_id: Some("glm-5.2".into()),
            provider: Some("zai".into()),
            input_tokens: 1_000_000,
            output_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            turns: 1,
            input_complete: 1,
            output_complete: 1,
            cache_read_complete: 0,
            cache_creation_complete: 0,
        }];
        let totals = totals_from_grain(&grain);
        assert!(approx(totals.cost, 1.40));
        assert!(totals.has_unpriced);
        assert!(totals.cost_excludes_cache);
        assert_eq!(totals.cache_read_tokens, None);
        assert_eq!(totals.cache_creation_tokens, None);
    }

    #[test]
    fn cache_incomplete_nonzero_total_with_zero_known_subtotal_has_no_price() {
        let row = UsageGrainRow {
            day: "".into(),
            model_id: Some("glm-5.2".into()),
            provider: Some("zai".into()),
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: Some(100),
            cache_read_tokens: None,
            cache_creation_tokens: None,
            turns: 1,
            input_complete: 1,
            output_complete: 1,
            cache_read_complete: 0,
            cache_creation_complete: 0,
        };
        let price = price_grain(&row, &ResolvedPricing::new());
        assert_eq!(price.cost, None);
        assert!(price.incomplete);
    }

    #[test]
    fn report_uses_resolved_declarative_provider_pricing() {
        let metadata = crate::providers::base::ProviderMetadata::with_models(
            "custom_acme",
            "Acme",
            "test",
            "acme-model",
            vec![crate::providers::base::ModelInfo::with_cost(
                "acme-model",
                32_000,
                0.000_002,
                0.000_008,
            )],
            "",
            vec![],
        );
        let pricing =
            crate::providers::pricing::pricing_from_provider_metadata(&metadata, "acme-model")
                .unwrap();
        let mut resolved = ResolvedPricing::new();
        resolved.insert(
            ("custom_acme".to_string(), "acme-model".to_string()),
            pricing,
        );
        let grain = vec![UsageGrainRow {
            day: "2026-07-13".into(),
            model_id: Some("acme-model".into()),
            provider: Some("custom_acme".into()),
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            total_tokens: Some(1_500_000),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 1,
            input_complete: 1,
            output_complete: 1,
            cache_read_complete: 1,
            cache_creation_complete: 1,
        }];
        let rows = rollup_report_with_pricing(&grain, UsageGroup::Model, &resolved);
        assert!(approx(rows[0].cost, 6.0));
        assert!(!rows[0].has_unpriced);
    }

    #[test]
    fn empty_span_is_exactly_zero_not_unknown() {
        let totals = totals_from_grain(&[]);
        assert_eq!(totals.total_tokens, Some(0));
        assert_eq!(totals.cache_read_tokens, Some(0));
        assert_eq!(totals.cache_creation_tokens, Some(0));
        assert_eq!(totals.cost, Some(0.0));
        assert!(!totals.has_unpriced);
        assert!(!totals.cost_excludes_cache);
    }

    #[test]
    fn rollup_prices_cache_buckets_and_flags_exclusion() {
        // Two grain rows:
        //  - Claude Sonnet (cache-priced): input 1M, output 200k, cache_read
        //    500k, cache_creation 100k. Hand-computed cost = 6.525 (see the
        //    pricing crate's model_cost_with_cache test).
        //  - zai glm-5.2 (no cache rate) with cache tokens: input 1M -> $1.40,
        //    cache omitted, so the bucket flags cost_excludes_cache.
        let grain = vec![
            UsageGrainRow {
                day: "2026-07-12".into(),
                model_id: Some("claude-sonnet-4-20250514".into()),
                provider: Some("anthropic".into()),
                input_tokens: 1_000_000,
                output_tokens: 200_000,
                total_tokens: Some(1_600_000),
                cache_read_tokens: Some(500_000),
                cache_creation_tokens: Some(100_000),
                turns: 1,
                input_complete: 1,
                output_complete: 1,
                cache_read_complete: 1,
                cache_creation_complete: 1,
            },
            UsageGrainRow {
                day: "2026-07-12".into(),
                model_id: Some("glm-5.2".into()),
                provider: Some("zai".into()),
                input_tokens: 1_000_000,
                output_tokens: 0,
                total_tokens: Some(1_900_000),
                cache_read_tokens: Some(900_000),
                cache_creation_tokens: Some(0),
                turns: 1,
                input_complete: 1,
                output_complete: 1,
                cache_read_complete: 1,
                cache_creation_complete: 1,
            },
        ];
        let day = &rollup_report(&grain, UsageGroup::Day)[0];
        assert_eq!(day.cache_read_tokens, Some(1_400_000));
        assert_eq!(day.cache_creation_tokens, Some(100_000));
        // 6.525 (sonnet incl. cache) + 1.40 (zai, cache excluded) = 7.925.
        assert!(approx(day.cost, 7.925), "got {:?}", day.cost);
        assert!(
            day.cost_excludes_cache,
            "zai carried cache with no cache rate"
        );

        let totals = totals_from_grain(&grain);
        assert_eq!(totals.cache_read_tokens, Some(1_400_000));
        assert_eq!(totals.cache_creation_tokens, Some(100_000));
        assert!(approx(totals.cost, 7.925));
        assert!(totals.cost_excludes_cache);
    }

    #[tokio::test]
    async fn empty_model_strings_collapse_into_unknown() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = seed_session_with_messages(&sm, 1).await;

        // Empty strings must be stored as NULL so they aggregate with genuine
        // unknowns rather than forming a "" group.
        sm.record_token_event(
            &session.id,
            Some(4),
            Some(1),
            5,
            Some(""),
            Some(""),
            None,
            None,
        )
        .await
        .unwrap();
        sm.record_token_event(&session.id, Some(6), Some(0), 6, None, None, None, None)
            .await
            .unwrap();

        let rows = sm.get_session_model_usage(&session.id).await.unwrap();
        assert_eq!(rows.len(), 1, "the empty-string turn folded into unknown");
        assert_eq!(rows[0].model_id, None);
        assert_eq!(rows[0].total_tokens, Some(11));
        assert_eq!(rows[0].turns, 2);
    }

    #[tokio::test]
    async fn global_model_usage_window_is_boundary_inclusive() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        // Creating a session initializes the schema on disk and gives us a
        // real (user-type) session row for the aggregation join.
        let session = seed_session_with_messages(&sm, 1).await;

        // Insert events at controlled timestamps by opening a second connection
        // to the same DB file — `record_token_event` always stamps `now`.
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        let opts = SqliteConnectOptions::new().filename(&db_path);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        for (ts, total) in [(1000_i64, 10_i64), (2000, 20), (3000, 30)] {
            sqlx::query(
                "INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens, billed_total_tokens, model_id, provider, session_type) VALUES (?, ?, ?, ?, ?, ?, 'm', 'p', 'user')",
            )
            .bind(&session.id)
            .bind(ts)
            .bind(total)
            .bind(0_i64)
            .bind(total)
            .bind(total)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool.close().await;

        // Both ends inclusive: [2000, 2000] catches exactly the ts=2000 row.
        let mid = sm.get_model_usage(2000, 2000).await.unwrap();
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].total_tokens, Some(20));
        assert_eq!(mid[0].turns, 1);

        // A window straddling only the middle row.
        let straddle = sm.get_model_usage(1500, 2500).await.unwrap();
        assert_eq!(straddle[0].total_tokens, Some(20));

        // Full span includes all three: from == first ts, to == last ts.
        let all = sm.get_model_usage(1000, 3000).await.unwrap();
        assert_eq!(all.len(), 1, "one model group");
        assert_eq!(all[0].total_tokens, Some(60));
        assert_eq!(all[0].turns, 3);

        // Just below the lowest ts excludes everything.
        let none = sm.get_model_usage(0, 999).await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn usage_report_includes_subagent_spend_and_excludes_hidden_sessions() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let user = seed_session_with_messages(&sm, 1).await;
        let subagent = sm
            .create_session(
                PathBuf::from("/tmp/sub"),
                "subagent".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let hidden = sm
            .create_session(
                PathBuf::from("/tmp/h"),
                "hidden".into(),
                SessionType::Hidden,
            )
            .await
            .unwrap();

        let t0: i64 = 1_700_000_000;
        let t2 = t0 + 2 * 86_400; // two days later

        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        let opts = SqliteConnectOptions::new().filename(&db_path);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();

        let insert = |sid: String,
                      ts: i64,
                      input: i64,
                      output: i64,
                      total: i64,
                      model: Option<&'static str>,
                      provider: Option<&'static str>,
                      pool: &sqlx::SqlitePool| {
            let pool = pool.clone();
            async move {
                sqlx::query("INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens, billed_total_tokens, model_id, provider, cache_read_tokens, cache_creation_tokens, session_type) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, 0, (SELECT session_type FROM sessions WHERE id = ?))")
                    .bind(sid.clone()).bind(ts).bind(input).bind(output).bind(total).bind(total).bind(model).bind(provider).bind(sid)
                    .execute(&pool).await.unwrap();
            }
        };

        // Day 0: a priced glm turn (1M input → $1.40) + an unknown turn.
        insert(
            user.id.clone(),
            t0,
            1_000_000,
            0,
            1_000_000,
            Some("glm-5.2"),
            Some("zai"),
            &pool,
        )
        .await;
        insert(user.id.clone(), t0, 100, 0, 100, None, None, &pool).await;
        // Day 2: a priced glm turn (1M output → $4.40).
        insert(
            user.id.clone(),
            t2,
            0,
            1_000_000,
            1_000_000,
            Some("glm-5.2"),
            Some("zai"),
            &pool,
        )
        .await;
        // Subagent work is hidden from the session list but is real spend.
        insert(
            subagent.id.clone(),
            t0,
            2_000_000,
            0,
            2_000_000,
            Some("glm-5.2"),
            Some("zai"),
            &pool,
        )
        .await;
        // Hidden bookkeeping sessions do not represent user workload.
        insert(
            hidden.id.clone(),
            t0,
            9_999_999,
            0,
            9_999_999,
            Some("glm-5.2"),
            Some("zai"),
            &pool,
        )
        .await;

        // The local-day strings are tz-dependent; ask SQLite so the assertion
        // holds in any timezone the test runs in.
        let day0: String = sqlx::query_scalar("SELECT date(?, 'unixepoch', 'localtime')")
            .bind(t0)
            .fetch_one(&pool)
            .await
            .unwrap();
        let day2: String = sqlx::query_scalar("SELECT date(?, 'unixepoch', 'localtime')")
            .bind(t2)
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;

        let report = sm
            .get_usage_report(t0 - 1, t2 + 1, UsageGroup::Day)
            .await
            .unwrap();
        assert_eq!(report.len(), 2, "two active local days, hidden excluded");

        let b0 = report
            .iter()
            .find(|r| r.date.as_deref() == Some(&day0))
            .unwrap();
        assert_eq!(
            b0.input_tokens, 3_000_100,
            "subagent included, hidden excluded"
        );
        assert_eq!(b0.output_tokens, 0);
        assert_eq!(b0.total_tokens, Some(3_000_100));
        assert_eq!(b0.turns, 3);
        assert!(
            matches!(b0.cost, Some(c) if (c - 4.20).abs() < 1e-9),
            "got {:?}",
            b0.cost
        );
        assert!(b0.has_unpriced, "the unknown turn flags the day partial");

        let b2 = report
            .iter()
            .find(|r| r.date.as_deref() == Some(&day2))
            .unwrap();
        assert_eq!(b2.total_tokens, Some(1_000_000));
        assert!(
            matches!(b2.cost, Some(c) if (c - 4.40).abs() < 1e-9),
            "got {:?}",
            b2.cost
        );
        assert!(!b2.has_unpriced);

        // Window that ends before day 2 drops that bucket entirely.
        let narrow = sm
            .get_usage_report(t0 - 1, t0 + 1, UsageGroup::Day)
            .await
            .unwrap();
        assert_eq!(narrow.len(), 1);
        assert_eq!(narrow[0].date.as_deref(), Some(day0.as_str()));
    }

    #[tokio::test]
    async fn usage_summary_month_to_date_respects_the_local_month_boundary() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let user = seed_session_with_messages(&sm, 1).await;

        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        let opts = SqliteConnectOptions::new().filename(&db_path);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();

        // Unix second of local midnight on the 1st of the current month. The
        // 'utc' modifier reads the wall-clock string as localtime, so this is a
        // real instant regardless of the runner's timezone.
        let month_start: i64 = sqlx::query_scalar(
            "SELECT CAST(strftime('%s', strftime('%Y-%m-01 00:00:00', 'now', 'localtime'), 'utc') AS INTEGER)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let first_of_month = month_start; // inside MTD
        let last_of_prev_month = month_start - 1; // one second earlier: previous month

        let insert = |ts: i64, input: i64, output: i64, total: i64, pool: &sqlx::SqlitePool| {
            let sid = user.id.clone();
            let pool = pool.clone();
            async move {
                sqlx::query("INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens, billed_total_tokens, model_id, provider, cache_read_tokens, cache_creation_tokens, session_type) VALUES (?, ?, ?, ?, ?, ?, 'glm-5.2', 'zai', 0, 0, 'user')")
                    .bind(sid).bind(ts).bind(input).bind(output).bind(total).bind(total)
                    .execute(&pool).await.unwrap();
            }
        };

        // In the current month: 1M input → $1.40.
        insert(first_of_month, 1_000_000, 0, 1_000_000, &pool).await;
        // One second into the previous month: 5M input → must be excluded from MTD.
        insert(last_of_prev_month, 5_000_000, 0, 5_000_000, &pool).await;
        pool.close().await;

        let summary = sm.get_usage_summary().await.unwrap();

        // MTD sees only the first-of-month row.
        assert_eq!(
            summary.month_to_date.total_tokens,
            Some(1_000_000),
            "the last-second-of-previous-month row must be excluded"
        );
        assert_eq!(summary.month_to_date.input_tokens, 1_000_000);
        assert!(
            matches!(summary.month_to_date.cost, Some(c) if (c - 1.40).abs() < 1e-9),
            "got {:?}",
            summary.month_to_date.cost
        );
        assert!(!summary.month_to_date.has_unpriced);

        // All-time sees both rows: 1M + 5M = 6M input → $8.40.
        assert_eq!(summary.all_time.total_tokens, Some(6_000_000));
        assert!(
            matches!(summary.all_time.cost, Some(c) if (c - 8.40).abs() < 1e-9),
            "got {:?}",
            summary.all_time.cost
        );

        // `month` is the current local YYYY-MM.
        assert_eq!(summary.month.len(), 7);
        assert_eq!(summary.month.chars().nth(4), Some('-'));
    }

    #[tokio::test]
    async fn migration_11_preserves_unknown_model_history_and_is_idempotent() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Build a DB frozen at schema v10: token_events WITHOUT the model_id /
        // provider columns, so opening the manager must run migration 11.
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();

            sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)")
                .execute(&pool).await.unwrap();
            for v in 1..=10 {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(v)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query(
                r#"CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
                    user_set_name BOOLEAN DEFAULT FALSE, session_type TEXT NOT NULL DEFAULT 'user', working_dir TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    extension_data TEXT DEFAULT '{}', total_tokens INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                    accumulated_total_tokens INTEGER, accumulated_input_tokens INTEGER, accumulated_output_tokens INTEGER,
                    schedule_id TEXT, workflow_json TEXT, user_workflow_values_json TEXT, provider_name TEXT,
                    model_config_json TEXT, diverged_from TEXT, external_key TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query(
                r#"CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
                    role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL,
                    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, tokens INTEGER, metadata_json TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            // v10-shape token_events: no model_id / provider columns.
            sqlx::query(
                r#"CREATE TABLE token_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, ts INTEGER NOT NULL,
                    input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER NOT NULL DEFAULT 0
                )"#,
            ).execute(&pool).await.unwrap();

            // Session S1 has only the session's final model/provider, which is
            // not valid attribution for earlier turns.
            sqlx::query(
                "INSERT INTO sessions (id, name, working_dir, provider_name, model_config_json) VALUES ('s1', 'has model', '/tmp/a', 'openai', '{\"model_name\":\"gpt-5\"}')",
            ).execute(&pool).await.unwrap();
            // Session S2 has no model config → its events stay unknown.
            sqlx::query(
                "INSERT INTO sessions (id, name, working_dir) VALUES ('s2', 'no model', '/tmp/b')",
            )
            .execute(&pool)
            .await
            .unwrap();

            sqlx::query("INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens) VALUES ('s1', 100, 8, 2, 10)")
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens) VALUES ('s1', 200, 40, 10, 50)")
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens) VALUES ('s2', 300, 5, 1, 6)")
                .execute(&pool).await.unwrap();
            pool.close().await;
        }

        // Opening the real manager triggers run_migrations → the `11 =>` arm.
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // Both sessions' legacy rows remain explicitly unattributed. Copying
        // S1's final model backward would fabricate history after a model switch.
        let s1 = sm.get_session_model_usage("s1").await.unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].model_id, None);
        assert_eq!(s1[0].provider, None);
        assert_eq!(s1[0].total_tokens, None);
        assert_eq!(s1[0].turns, 2);

        // S2 has no stored model, so its row stays unknown (NULL model_id).
        let s2 = sm.get_session_model_usage("s2").await.unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].model_id, None);
        assert_eq!(s2[0].provider, None);
        assert_eq!(s2[0].total_tokens, None);

        // Idempotency: re-applying migration 11 on the already-migrated DB is a
        // no-op — the pragma guards skip the ADD COLUMNs and neither invocation
        // fabricates model attribution.
        let db_path2 = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        let opts = SqliteConnectOptions::new().filename(&db_path2);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        assert_eq!(
            SessionStorage::get_schema_version(&pool).await.unwrap(),
            CURRENT_SCHEMA_VERSION,
            "v10 must run usage v11/v12 before loop v13-v16"
        );
        SessionStorage::apply_migration(&pool, 11).await.unwrap();
        SessionStorage::apply_migration(&pool, 11).await.unwrap();
        pool.close().await;

        let s1_again = sm.get_session_model_usage("s1").await.unwrap();
        assert_eq!(s1_again[0].total_tokens, None);
        assert_eq!(s1_again[0].model_id, None);
        let s2_again = sm.get_session_model_usage("s2").await.unwrap();
        assert_eq!(s2_again[0].model_id, None);
    }

    #[tokio::test]
    async fn migration_12_adds_nullable_accounting_columns_and_is_idempotent() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Build a DB frozen at schema v11: token_events WITH model_id/provider
        // but WITHOUT the cache columns, so opening the manager runs migration 12.
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();

            sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)")
                .execute(&pool).await.unwrap();
            for v in 1..=11 {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(v)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query(
                r#"CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
                    user_set_name BOOLEAN DEFAULT FALSE, session_type TEXT NOT NULL DEFAULT 'user', working_dir TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    extension_data TEXT DEFAULT '{}', total_tokens INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                    accumulated_total_tokens INTEGER, accumulated_input_tokens INTEGER, accumulated_output_tokens INTEGER,
                    schedule_id TEXT, workflow_json TEXT, user_workflow_values_json TEXT, provider_name TEXT,
                    model_config_json TEXT, diverged_from TEXT, external_key TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query(
                r#"CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
                    role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL,
                    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, tokens INTEGER, metadata_json TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            // v11-shape token_events: model_id/provider but no cache columns.
            sqlx::query(
                r#"CREATE TABLE token_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, ts INTEGER NOT NULL,
                    input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER NOT NULL DEFAULT 0,
                    model_id TEXT, provider TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO sessions (id, name, working_dir) VALUES ('s1', 'legacy', '/tmp/a')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens, model_id, provider) VALUES ('s1', 100, 8, 2, 10, 'm', 'p')")
                .execute(&pool).await.unwrap();
            pool.close().await;
        }

        // Opening the manager triggers run_migrations → the `12 =>` arm, which
        // adds nullable accounting columns; the pre-existing legacy row remains
        // unknown instead of being rewritten as measured zero.
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let rows = sm.get_session_model_usage("s1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, None);
        assert_eq!(rows[0].provider, None);
        assert_eq!(rows[0].total_tokens, None);
        assert_eq!(rows[0].cache_read_tokens, None);
        assert_eq!(rows[0].cache_creation_tokens, None);

        // A new turn records real cache values through the added columns.
        sm.record_token_event(
            "s1",
            Some(1),
            Some(1),
            902,
            Some("m-new"),
            Some("p"),
            Some(700),
            Some(200),
        )
        .await
        .unwrap();
        let rows = sm.get_session_model_usage("s1").await.unwrap();
        let current = rows
            .iter()
            .find(|row| row.model_id.as_deref() == Some("m-new"))
            .unwrap();
        assert_eq!(current.total_tokens, Some(902));
        assert_eq!(current.cache_read_tokens, Some(700));
        assert_eq!(current.cache_creation_tokens, Some(200));

        // Idempotency: re-applying migration 12 twice more is a no-op (the pragma
        // guards skip the ADD COLUMNs) and does not disturb the recorded data.
        let opts = SqliteConnectOptions::new().filename(&db_path);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        SessionStorage::apply_migration(&pool, 12).await.unwrap();
        SessionStorage::apply_migration(&pool, 12).await.unwrap();
        for column in [
            "billed_total_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "event_key",
            "session_type",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('token_events') WHERE name = ?1",
            )
            .bind(column)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(exists, 1, "missing migrated column {column}");
        }
        let event_index: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_token_events_event_key'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(event_index, 1);
        pool.close().await;

        let rows = sm.get_session_model_usage("s1").await.unwrap();
        let legacy = rows.iter().find(|row| row.model_id.is_none()).unwrap();
        assert_eq!(legacy.total_tokens, None);
        assert_eq!(legacy.cache_read_tokens, None);
        assert_eq!(legacy.cache_creation_tokens, None);
        let current = rows
            .iter()
            .find(|row| row.model_id.as_deref() == Some("m-new"))
            .unwrap();
        assert_eq!(current.total_tokens, Some(902));
        assert_eq!(current.cache_read_tokens, Some(700));
        assert_eq!(current.cache_creation_tokens, Some(200));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn experimental_loop_v11_through_v14_shapes_upgrade_without_loss() {
        for legacy_version in 11..=14 {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir
                .path()
                .join(format!("experimental-v{legacy_version}.db"));
            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();

            sqlx::query(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO schema_version (version) VALUES (?1)")
                .bind(legacy_version)
                .execute(&pool)
                .await
                .unwrap();

            let branch_column = if legacy_version >= 12 {
                ", branch_point_msg_uid TEXT"
            } else {
                ""
            };
            sqlx::query(&format!(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, session_type TEXT NOT NULL DEFAULT 'user'{branch_column})"
            ))
            .execute(&pool)
            .await
            .unwrap();
            let message_uid_column = if legacy_version >= 12 {
                ", msg_uid TEXT"
            } else {
                ""
            };
            sqlx::query(&format!(
                "CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, content_json TEXT NOT NULL, metadata_json TEXT{message_uid_column})"
            ))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "CREATE TABLE token_events (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, ts INTEGER NOT NULL, input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER NOT NULL DEFAULT 0)",
            )
            .execute(&pool)
            .await
            .unwrap();

            if legacy_version >= 12 {
                sqlx::query(
                    "INSERT INTO sessions (id, session_type, branch_point_msg_uid) VALUES ('s1', 'user', 'legacy-anchor')",
                )
                .execute(&pool)
                .await
                .unwrap();
            } else {
                sqlx::query("INSERT INTO sessions (id, session_type) VALUES ('s1', 'user')")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            let content_json =
                serde_json::to_string(&vec![MessageContent::text("legacy searchable")]).unwrap();
            if legacy_version >= 12 {
                sqlx::query(
                    "INSERT INTO messages (session_id, content_json, msg_uid) VALUES ('s1', ?1, 'legacy-uid')",
                )
                .bind(&content_json)
                .execute(&pool)
                .await
                .unwrap();
            } else {
                sqlx::query("INSERT INTO messages (session_id, content_json) VALUES ('s1', ?1)")
                    .bind(&content_json)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query(
                "INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens) VALUES ('s1', 100, 8, 2, 10)",
            )
            .execute(&pool)
            .await
            .unwrap();

            SessionStorage::create_checkpoints_table(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO checkpoints (id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha) VALUES ('cp1', 's1', 0, 100, 'pre_step', 'commit', 'tree')",
            )
            .execute(&pool)
            .await
            .unwrap();
            if legacy_version >= 13 {
                sqlx::query(MESSAGES_FTS_DDL).execute(&pool).await.unwrap();
                sqlx::query(MESSAGES_FTS_INSERT)
                    .bind("legacy searchable")
                    .bind("s1")
                    .bind(1_i64)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            if legacy_version >= 14 {
                SessionStorage::create_message_blobs_table(&pool)
                    .await
                    .unwrap();
                sqlx::query(
                    "INSERT INTO message_blobs (blob_uid, session_id, created_at, bytes, content) VALUES ('blob1', 's1', 100, 7, 'payload')",
                )
                .execute(&pool)
                .await
                .unwrap();
            }

            SessionStorage::run_migrations(&pool).await.unwrap();

            assert_eq!(
                SessionStorage::get_schema_version(&pool).await.unwrap(),
                CURRENT_SCHEMA_VERSION,
                "experimental v{legacy_version} did not reach v16"
            );
            for column in [
                "model_id",
                "provider",
                "cache_read_tokens",
                "cache_creation_tokens",
                "billed_total_tokens",
                "event_key",
                "session_type",
            ] {
                assert!(
                    SessionStorage::table_has_column(&pool, "token_events", column)
                        .await
                        .unwrap(),
                    "experimental v{legacy_version} missed usage column {column}"
                );
            }
            let usage: (i64, Option<String>) = sqlx::query_as(
                "SELECT total_tokens, session_type FROM token_events WHERE session_id = 's1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(usage, (10, Some("user".into())));

            let checkpoint_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM checkpoints WHERE id = 'cp1'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(checkpoint_count, 1);
            let message_uid: String =
                sqlx::query_scalar("SELECT msg_uid FROM messages WHERE id = 1")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                message_uid,
                if legacy_version >= 12 {
                    "legacy-uid"
                } else {
                    "m1"
                }
            );
            let branch_point: Option<String> =
                sqlx::query_scalar("SELECT branch_point_msg_uid FROM sessions WHERE id = 's1'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                branch_point.as_deref(),
                (legacy_version >= 12).then_some("legacy-anchor")
            );
            let fts_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM messages_fts WHERE session_id = 's1' AND messages_fts MATCH 'legacy'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(fts_count, 1, "FTS rows were duplicated or lost");
            let blob_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM message_blobs WHERE session_id = 's1'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(blob_count, i64::from(legacy_version >= 14));
        }
    }

    #[tokio::test]
    async fn pr13_v12_database_reconciles_usage_and_adds_loop_schema() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Early v12 builds had model/cache columns and marked the schema as
        // current, but lacked billed totals, durable event keys, and event-level
        // session types. They also copied final-session attribution backward and
        // could materialize unknown cache values as zero.
        {
            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO schema_version (version) VALUES (12)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, session_type TEXT NOT NULL DEFAULT 'user')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, content_json TEXT NOT NULL DEFAULT '[]', metadata_json TEXT)",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                r#"CREATE TABLE token_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    ts INTEGER NOT NULL,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    model_id TEXT,
                    provider TEXT,
                    cache_read_tokens INTEGER,
                    cache_creation_tokens INTEGER
                )"#,
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO sessions (id, session_type) VALUES ('s1', 'user')")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO token_events (session_id, ts, input_tokens, output_tokens, total_tokens, model_id, provider, cache_read_tokens, cache_creation_tokens) VALUES ('s1', 100, 8, 2, 10, 'fabricated-final-model', 'fabricated-provider', 0, 0)",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }

        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let rows = sm.get_session_model_usage("s1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, None);
        assert_eq!(rows[0].provider, None);
        assert_eq!(rows[0].total_tokens, None);
        assert_eq!(rows[0].cache_read_tokens, None);
        assert_eq!(rows[0].cache_creation_tokens, None);

        let pool = sm.storage.pool().await.unwrap();
        SessionStorage::reconcile_usage_schema(pool).await.unwrap();
        SessionStorage::reconcile_usage_schema(pool).await.unwrap();
        SessionStorage::reconcile_loop_schema(pool).await.unwrap();
        SessionStorage::reconcile_loop_schema(pool).await.unwrap();
        let version: i32 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        for table in ["checkpoints", "messages_fts", "message_blobs"] {
            let exists: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?1")
                    .bind(table)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            assert_eq!(exists, 1, "missing reconciled table {table}");
        }
        assert!(
            SessionStorage::table_has_column(pool, "messages", "msg_uid")
                .await
                .unwrap()
        );
        assert!(
            SessionStorage::table_has_column(pool, "sessions", "branch_point_msg_uid")
                .await
                .unwrap()
        );
        type ReconciledUsageRow = (
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<String>,
        );
        let raw: ReconciledUsageRow = sqlx::query_as(
            "SELECT model_id, provider, cache_read_tokens, cache_creation_tokens, billed_total_tokens, session_type FROM token_events WHERE session_id = 's1'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(raw, (None, None, None, None, None, Some("user".into())));

        let before_delete = sm.get_usage_summary().await.unwrap();
        assert_eq!(before_delete.all_time.turns, 1);
        sm.delete_session("s1").await.unwrap();
        let after_delete = sm.get_usage_summary().await.unwrap();
        assert_eq!(after_delete.all_time.turns, 1);
        assert_eq!(after_delete.all_time.total_tokens, None);
    }

    #[tokio::test]
    async fn test_get_token_counts_matches_get_session() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = seed_session_with_messages(&sm, 2).await;
        sm.update(&session.id)
            .total_tokens(Some(4321))
            .input_tokens(Some(4000))
            .output_tokens(Some(321))
            .accumulated_total_tokens(Some(9999))
            .accumulated_input_tokens(Some(9000))
            .accumulated_output_tokens(Some(999))
            .apply()
            .await
            .unwrap();

        // The lightweight token-only query must return exactly the same token
        // counters that the full `get_session` exposes (it just skips the
        // COUNT(*) and metadata columns).
        let full = sm.get_session(&session.id, false).await.unwrap();
        let counts = sm.get_token_counts(&session.id).await.unwrap();

        assert_eq!(counts.total_tokens, full.total_tokens);
        assert_eq!(counts.input_tokens, full.input_tokens);
        assert_eq!(counts.output_tokens, full.output_tokens);
        assert_eq!(
            counts.accumulated_total_tokens,
            full.accumulated_total_tokens
        );
        assert_eq!(
            counts.accumulated_input_tokens,
            full.accumulated_input_tokens
        );
        assert_eq!(
            counts.accumulated_output_tokens,
            full.accumulated_output_tokens
        );
        assert_eq!(counts.total_tokens, Some(4321));
        assert_eq!(counts.accumulated_total_tokens, Some(9999));
    }

    /// BR-52: both ways of turning stored counters into the `TokenState` clients
    /// see must agree, since one seeds the SSE stream (from the session row the
    /// route already read) and the other refreshes it (from the agent's own
    /// boundary read). If they disagreed, the token readout would jump.
    #[tokio::test]
    async fn token_state_from_counts_and_from_session_agree() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = seed_session_with_messages(&sm, 1).await;
        sm.update(&session.id)
            .total_tokens(Some(4321))
            .input_tokens(Some(4000))
            .output_tokens(Some(321))
            .accumulated_total_tokens(Some(9999))
            .accumulated_input_tokens(Some(9000))
            .accumulated_output_tokens(Some(999))
            .apply()
            .await
            .unwrap();

        let full = sm.get_session(&session.id, false).await.unwrap();
        let from_session = TokenState::from(&full);
        let from_counts = TokenState::from(sm.get_token_counts(&session.id).await.unwrap());

        assert_eq!(from_session.total_tokens, 4321);
        assert_eq!(from_session.input_tokens, 4000);
        assert_eq!(from_session.output_tokens, 321);
        assert_eq!(from_session.accumulated_total_tokens, 9999);
        assert_eq!(from_session.accumulated_input_tokens, 9000);
        assert_eq!(from_session.accumulated_output_tokens, 999);

        assert_eq!(from_counts.total_tokens, from_session.total_tokens);
        assert_eq!(from_counts.input_tokens, from_session.input_tokens);
        assert_eq!(from_counts.output_tokens, from_session.output_tokens);
        assert_eq!(
            from_counts.accumulated_total_tokens,
            from_session.accumulated_total_tokens
        );
        assert_eq!(
            from_counts.accumulated_input_tokens,
            from_session.accumulated_input_tokens
        );
        assert_eq!(
            from_counts.accumulated_output_tokens,
            from_session.accumulated_output_tokens
        );
    }

    /// A brand-new session has NULL counters; both conversions must read as zero
    /// rather than panicking or surfacing a negative default.
    #[tokio::test]
    async fn token_state_of_a_fresh_session_is_zero() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = sm
            .create_session(
                PathBuf::from("/tmp/fresh"),
                "Fresh".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let state = TokenState::from(&session);
        assert_eq!(state.total_tokens, 0);
        assert_eq!(state.accumulated_total_tokens, 0);

        let state = TokenState::from(sm.get_token_counts(&session.id).await.unwrap());
        assert_eq!(state.total_tokens, 0);
        assert_eq!(state.accumulated_total_tokens, 0);
    }

    #[tokio::test]
    async fn test_diverge_empty_conversation() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = sm
            .create_session(
                PathBuf::from("/tmp/empty"),
                "Empty".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let diverged = sm
            .copy_session(&original.id, "Branch of empty".to_string())
            .await
            .unwrap();

        assert_ne!(diverged.id, original.id);
        assert_eq!(diverged.message_count, 0);
        assert_eq!(diverged.name, "Branch of empty");
    }

    #[tokio::test]
    async fn test_diverge_of_a_diverge_chains() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 2).await;
        let first = sm
            .copy_session(&original.id, "First branch".to_string())
            .await
            .unwrap();
        let second = sm
            .copy_session(&first.id, "Second branch".to_string())
            .await
            .unwrap();

        // Three distinct sessions, all sharing the same history.
        let ids: std::collections::HashSet<_> =
            [&original.id, &first.id, &second.id].into_iter().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(second.message_count, 4);
        assert_eq!(sm.list_sessions().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_diverge_nonexistent_session_errors() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let result = sm
            .copy_session("does_not_exist", "Branch".to_string())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_diverge_produces_unique_ids() {
        let temp_dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));

        let original = seed_session_with_messages(&sm, 2).await;

        let mut handles = vec![];
        for i in 0..NUM_CONCURRENT_SESSIONS {
            let sm = Arc::clone(&sm);
            let oid = original.id.clone();
            handles.push(tokio::spawn(async move {
                sm.copy_session(&oid, format!("Branch {i}"))
                    .await
                    .unwrap()
                    .id
            }));
        }

        let mut ids = std::collections::HashSet::new();
        for h in handles {
            ids.insert(h.await.unwrap());
        }
        // Every concurrent diverge yields a unique id, none colliding with the
        // original.
        assert_eq!(ids.len(), NUM_CONCURRENT_SESSIONS as usize);
        assert!(!ids.contains(&original.id));

        // Original + all branches persisted; original still has its 4 messages.
        assert_eq!(
            sm.list_sessions().await.unwrap().len(),
            NUM_CONCURRENT_SESSIONS as usize + 1
        );
        assert_eq!(
            sm.get_session(&original.id, true)
                .await
                .unwrap()
                .message_count,
            4
        );
    }

    // ── Branch naming + lineage (diverge_session) ───────────────────────────

    #[test]
    fn test_strip_branch_suffix() {
        assert_eq!(strip_branch_suffix("Foo"), "Foo");
        assert_eq!(strip_branch_suffix("Foo (branch 1)"), "Foo");
        assert_eq!(strip_branch_suffix("Foo (branch 42)"), "Foo");
        // Only strips one level / a real numeric suffix.
        assert_eq!(
            strip_branch_suffix("Foo (branch 1) (branch 2)"),
            "Foo (branch 1)"
        );
        assert_eq!(strip_branch_suffix("Foo (branch)"), "Foo (branch)");
        assert_eq!(strip_branch_suffix("Foo (branch abc)"), "Foo (branch abc)");
        // A name that merely contains the word branch is untouched.
        assert_eq!(strip_branch_suffix("My branch plan"), "My branch plan");
    }

    #[test]
    fn test_is_default_session_name() {
        assert!(is_default_session_name(""));
        assert!(is_default_session_name("   "));
        assert!(is_default_session_name("New chat"));
        assert!(is_default_session_name("New Session"));
        assert!(is_default_session_name("new session"));
        assert!(is_default_session_name("CLI Session"));
        assert!(is_default_session_name("New session 3"));
        assert!(is_default_session_name("Session 12"));
        assert!(!is_default_session_name("Glycolysis explained"));
        assert!(!is_default_session_name("Session about sessions"));
    }

    #[test]
    fn default_name_canonicalization_respects_an_explicit_user_name() {
        assert_eq!(
            canonical_session_name("New Session".to_string(), false),
            DEFAULT_SESSION_NAME
        );
        assert_eq!(
            canonical_session_name("New Session".to_string(), true),
            "New Session"
        );
    }

    #[tokio::test]
    async fn test_diverge_session_names_branches_and_sets_lineage() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 2).await;
        sm.update(&original.id)
            .user_provided_name("Glycolysis")
            .apply()
            .await
            .unwrap();

        let b1 = sm.diverge_session(&original.id, None, None).await.unwrap();
        let b2 = sm.diverge_session(&original.id, None, None).await.unwrap();

        // Sibling-numbered, collision-free names.
        assert_eq!(b1.name, "Glycolysis (branch 1)");
        assert_eq!(b2.name, "Glycolysis (branch 2)");
        // Lineage recorded; original has none.
        assert_eq!(b1.diverged_from.as_deref(), Some(original.id.as_str()));
        assert_eq!(b2.diverged_from.as_deref(), Some(original.id.as_str()));
        assert_eq!(
            sm.get_session(&original.id, false)
                .await
                .unwrap()
                .diverged_from,
            None
        );
        // Full history carried over.
        assert_eq!(b1.message_count, 4);
    }

    #[tokio::test]
    async fn test_edit_diverge_uses_branch_naming_lineage_and_prefix() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let original = sm
            .create_session(
                PathBuf::from("/tmp/edit_diverge"),
                "Original".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        sm.update(&original.id)
            .user_provided_name("Weather analysis")
            .apply()
            .await
            .unwrap();
        for message in [
            umsg(10, "first question"),
            amsg(11, "first answer"),
            umsg(20, "message to edit"),
            amsg(21, "answer to replace"),
        ] {
            sm.add_message(&original.id, &message).await.unwrap();
        }

        let loaded = sm.get_session(&original.id, true).await.unwrap();
        let expected_branch_point = loaded.conversation.as_ref().unwrap().messages()[1]
            .id
            .clone();

        let first = sm.diverge_session_for_edit(&original.id, 20).await.unwrap();
        let second = sm.diverge_session_for_edit(&original.id, 20).await.unwrap();

        assert_eq!(first.name, "Weather analysis (branch 1)");
        assert_eq!(second.name, "Weather analysis (branch 2)");
        assert!(first.user_set_name);
        assert_eq!(first.diverged_from.as_deref(), Some(original.id.as_str()));
        assert_eq!(first.branch_point_msg_uid, expected_branch_point);
        assert_eq!(first.message_count, 2);
        assert_eq!(
            first
                .conversation
                .as_ref()
                .unwrap()
                .messages()
                .iter()
                .map(Message::as_concat_text)
                .collect::<Vec<_>>(),
            vec!["first question", "first answer"]
        );
        assert_eq!(
            sm.get_session(&original.id, true)
                .await
                .unwrap()
                .message_count,
            4
        );
    }

    #[tokio::test]
    async fn test_diverge_of_a_branch_flattens_numbering() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 1).await;
        sm.update(&original.id)
            .user_provided_name("Topic")
            .apply()
            .await
            .unwrap();

        let b1 = sm.diverge_session(&original.id, None, None).await.unwrap();
        assert_eq!(b1.name, "Topic (branch 1)");

        // Diverging the *branch* strips its suffix and continues the family
        // count rather than nesting "(branch 1) (branch 1)".
        let b2 = sm.diverge_session(&b1.id, None, None).await.unwrap();
        assert_eq!(b2.name, "Topic (branch 2)");
        // Its lineage points at the immediate parent (the branch).
        assert_eq!(b2.diverged_from.as_deref(), Some(b1.id.as_str()));
    }

    #[tokio::test]
    async fn test_diverge_placeholder_name_derives_from_conversation() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // Session left with the default placeholder name.
        let session = sm
            .create_session(
                PathBuf::from("/tmp/ph"),
                "New Session".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(
            &session.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("Explain the citric acid cycle")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let branch = sm.diverge_session(&session.id, None, None).await.unwrap();
        // Name derives from the first user message, not "New chat".
        assert!(
            branch.name.starts_with("Explain the citric acid cycle"),
            "unexpected branch name: {}",
            branch.name
        );
        assert!(branch.name.ends_with("(branch 1)"));
    }

    #[tokio::test]
    async fn test_diverge_preserves_a_user_named_new_session() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/user-named-new-session"),
                DEFAULT_SESSION_NAME.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.update(&session.id)
            .user_provided_name("New Session")
            .apply()
            .await
            .unwrap();
        sm.add_message(&session.id, &umsg(10, "Keep my chosen title"))
            .await
            .unwrap();

        let branch = sm.diverge_session(&session.id, None, None).await.unwrap();

        assert_eq!(branch.name, "New Session (branch 1)");
    }

    #[tokio::test]
    async fn session_summaries_distinguish_legacy_defaults_from_user_names() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "New Session".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(&session.id, &umsg(10, "hello"))
            .await
            .unwrap();

        let summary = sm
            .list_session_summaries(10, 0, false, false)
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == session.id)
            .unwrap();
        assert_eq!(summary.name, DEFAULT_SESSION_NAME);
        assert!(!summary.user_set_name);

        sm.update(&session.id)
            .user_provided_name("New Session")
            .apply()
            .await
            .unwrap();
        let summary = sm
            .list_session_summaries(10, 0, false, false)
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == session.id)
            .unwrap();
        assert_eq!(summary.name, "New Session");
        assert!(summary.user_set_name);
    }

    #[test]
    fn legacy_session_summary_json_defaults_user_set_name_to_false() {
        let summary: SessionSummary = serde_json::from_value(serde_json::json!({
            "id": "legacy",
            "working_dir": "/tmp",
            "name": "New Session",
            "created_at": "2026-08-18T00:00:00Z",
            "updated_at": "2026-08-18T00:00:00Z",
            "message_count": 1
        }))
        .unwrap();

        assert!(!summary.user_set_name);
    }

    #[tokio::test]
    async fn test_diverge_custom_name_overrides() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 1).await;
        let branch = sm
            .diverge_session(&original.id, Some("  Hand Picked  ".to_string()), None)
            .await
            .unwrap();
        // Trimmed, used verbatim (no "(branch N)" suffix), lineage still set.
        assert_eq!(branch.name, "Hand Picked");
        assert_eq!(branch.diverged_from.as_deref(), Some(original.id.as_str()));
    }

    #[tokio::test]
    async fn test_diverge_branch_name_survives_like_wildcards() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = seed_session_with_messages(&sm, 1).await;
        // A name containing SQL LIKE metacharacters must not break sibling
        // counting.
        sm.update(&original.id)
            .user_provided_name("100%_done")
            .apply()
            .await
            .unwrap();

        let b1 = sm.diverge_session(&original.id, None, None).await.unwrap();
        let b2 = sm.diverge_session(&original.id, None, None).await.unwrap();
        assert_eq!(b1.name, "100%_done (branch 1)");
        assert_eq!(b2.name, "100%_done (branch 2)");
    }

    // ── Branch trimming (start exactly from the last complete answer) ───────

    /// The branch conversation a pair of anchors produces, dropping
    /// `branch_conversation_at`'s honoured-anchor half.
    fn branched(
        conv: &Conversation,
        anchor_uid: Option<&str>,
        anchor_ms: Option<i64>,
    ) -> Conversation {
        branch_conversation_at(conv, anchor_uid, anchor_ms).0
    }

    fn umsg(created: i64, text: &str) -> Message {
        Message {
            id: None,
            role: Role::User,
            created,
            content: vec![MessageContent::text(text)],
            metadata: Default::default(),
        }
    }
    fn amsg(created: i64, text: &str) -> Message {
        Message {
            id: None,
            role: Role::Assistant,
            created,
            content: vec![MessageContent::text(text)],
            metadata: Default::default(),
        }
    }
    fn atool(created: i64) -> Message {
        let mut m = Message::assistant().with_tool_request(
            "call_1",
            Ok(rmcp::model::CallToolRequestParams {
                task: None,
                name: "shell".into(),
                arguments: None,
                meta: None,
            }),
        );
        m.created = created;
        m
    }

    #[test]
    fn test_trim_keeps_through_last_complete_answer() {
        let conv = Conversation::new_unvalidated(vec![
            umsg(1, "q1"),
            amsg(2, "a1"),
            umsg(3, "q2"),
            amsg(4, "a2"),
        ]);
        let t = branched(&conv, None, None);
        assert_eq!(t.messages().len(), 4);
        assert_eq!(t.messages().last().unwrap().as_concat_text(), "a2");
    }

    #[test]
    fn test_trim_drops_trailing_unanswered_question() {
        // The reported bug: diverge fired while the agent was still generating
        // the answer to q2, so the DB has q2 persisted with no answer yet.
        let conv = Conversation::new_unvalidated(vec![umsg(1, "q1"), amsg(2, "a1"), umsg(3, "q2")]);
        let t = branched(&conv, None, None);
        assert_eq!(t.messages().len(), 2);
        assert_eq!(t.messages().last().unwrap().as_concat_text(), "a1");
    }

    #[test]
    fn test_trim_drops_trailing_empty_assistant_and_tool_call() {
        // Mid tool-call: assistant("") then a pending tool request, no final
        // answer yet → branch ends at the previous complete answer.
        let conv = Conversation::new_unvalidated(vec![
            umsg(1, "q1"),
            amsg(2, "a1"),
            umsg(3, "q2"),
            amsg(4, ""),
            atool(5),
        ]);
        let t = branched(&conv, None, None);
        assert_eq!(t.messages().len(), 2);
        assert_eq!(t.messages().last().unwrap().as_concat_text(), "a1");
    }

    #[test]
    fn test_trim_anchor_bounds_branch_to_clicked_answer() {
        let conv = Conversation::new_unvalidated(vec![
            umsg(10, "q1"),
            amsg(20, "a1"),
            umsg(30, "q2"),
            amsg(40, "a2"),
        ]);
        // Per-message Diverge button clicked on a1 (created=20).
        let t = branched(&conv, None, Some(20));
        assert_eq!(t.messages().len(), 2);
        assert_eq!(t.messages().last().unwrap().as_concat_text(), "a1");
    }

    #[test]
    fn test_trim_empty_when_no_complete_answer() {
        // Diverged before the first reply landed: only an unanswered question.
        let conv = Conversation::new_unvalidated(vec![umsg(1, "q1")]);
        let t = branched(&conv, None, None);
        assert!(t.messages().is_empty());
    }

    // ── Issue #167: a branch anchor that cannot be trusted must widen the
    //    branch, never narrow it ────────────────────────────────────────────

    /// A realistic tool-using exchange: the assistant says what it is about to
    /// do, calls a tool, reads the result, then answers. `id_prefix` names the
    /// two assistant *text* rows, which are the only ones the desktop renders a
    /// Branch control on.
    fn tool_using_exchange(base: i64, id_prefix: &str, question: &str) -> Vec<Message> {
        let call = format!("{id_prefix}_call");
        let mut preamble = Message::assistant()
            .with_text("Let me look.")
            .with_id(format!("{id_prefix}-preamble"));
        preamble.created = base + 1;
        let mut request = Message::assistant()
            .with_id(format!("{id_prefix}-req"))
            .with_tool_request(
                call.clone(),
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: "shell".into(),
                    arguments: None,
                    meta: None,
                }),
            );
        request.created = base + 2;
        let mut response = Message::user()
            .with_id(format!("{id_prefix}-resp"))
            .with_tool_response(
                call,
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("ok"),
                ])),
            );
        response.created = base + 3;
        let mut answer = Message::assistant()
            .with_text(format!("answer to {question}"))
            .with_id(format!("{id_prefix}-answer"));
        answer.created = base + 4;

        let mut q = umsg(base, question);
        q.id = Some(format!("{id_prefix}-q"));
        vec![q, preamble, request, response, answer]
    }

    /// Three tool-using exchanges — the shape #167 was reported against.
    fn tool_using_conversation() -> Conversation {
        Conversation::new_unvalidated(
            tool_using_exchange(100, "e1", "q1")
                .into_iter()
                .chain(tool_using_exchange(200, "e2", "q2"))
                .chain(tool_using_exchange(300, "e3", "q3"))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn trim_anchored_on_a_tool_using_answer_keeps_the_whole_history_before_it() {
        let conv = tool_using_conversation();
        // Branch clicked on the LAST answer: everything is carried over.
        let t = branched(&conv, Some("e3-answer"), Some(304));
        assert_eq!(t.messages().len(), 15);
        assert_eq!(
            t.messages().last().unwrap().as_concat_text(),
            "answer to q3"
        );

        // Branch clicked on the MIDDLE answer: the branch ends there, and the
        // tool request/response pair of that exchange comes with it.
        let t = branched(&conv, Some("e2-answer"), Some(204));
        assert_eq!(t.messages().len(), 10);
        assert_eq!(
            t.messages().last().unwrap().as_concat_text(),
            "answer to q2"
        );
    }

    #[test]
    fn trim_refuses_an_anchor_id_that_resolves_older_than_its_own_timestamp() {
        // Issue #167, reproduced. The user pressed Branch on the last answer
        // (created 304) but the id their client held for that message is one
        // storage had given to the FIRST reply. Resolving the id alone cuts the
        // branch down to the first exchange and silently discards the chat.
        let conv = tool_using_conversation();
        let t = branched(&conv, Some("e1-preamble"), Some(304));

        assert_eq!(
            t.messages().len(),
            15,
            "a stale id must widen the branch to the whole conversation, not \
             truncate it at the message the id happens to name"
        );
        assert_eq!(
            t.messages().last().unwrap().as_concat_text(),
            "answer to q3"
        );
    }

    #[test]
    fn trim_honours_an_anchor_row_created_after_the_clicked_message() {
        // One streamed reply is stored as several rows and only the first keeps
        // the id the client was shown; the rest are minted as they are built, so
        // a row can carry a `created` a second LATER than the message on screen.
        // That is an honest anchor and must still cut the branch.
        let conv = tool_using_conversation();
        let t = branched(&conv, Some("e2-answer"), Some(203));
        assert_eq!(t.messages().len(), 10);
        assert_eq!(
            t.messages().last().unwrap().as_concat_text(),
            "answer to q2"
        );
    }

    #[test]
    fn trim_keeps_everything_when_the_anchor_id_is_unresolvable() {
        // The desktop's live view is structurally short of the store, so an id
        // it holds may name no stored row at all. That is not evidence about
        // where the user clicked — the old code degraded to the timestamp and
        // cut there anyway.
        let conv = tool_using_conversation();
        let t = branched(&conv, Some("never-stored"), Some(104));
        assert_eq!(t.messages().len(), 15);
        assert_eq!(
            t.messages().last().unwrap().as_concat_text(),
            "answer to q3"
        );
    }

    #[test]
    fn trim_resolves_a_duplicated_anchor_id_to_the_last_match() {
        // Storage enforces UNIQUE(session_id, msg_uid), but a conversation
        // assembled in memory can carry a decoder's reused id. `position()`
        // took the FIRST match, which is the narrowest — and worst — reading.
        let mut first = Message::assistant().with_text("a1").with_id("dup");
        first.created = 10;
        let mut second = Message::assistant().with_text("a2").with_id("dup");
        second.created = 30;
        let conv = Conversation::new_unvalidated(vec![
            umsg(1, "q1"),
            first,
            umsg(20, "q2"),
            second,
            umsg(40, "q3"),
            amsg(50, "a3"),
        ]);

        let t = branched(&conv, Some("dup"), None);
        assert_eq!(t.messages().len(), 4);
        assert_eq!(t.messages().last().unwrap().as_concat_text(), "a2");
    }

    #[test]
    fn trim_by_timestamp_alone_cuts_a_prefix_rather_than_filtering() {
        // `created` is not monotonic in a real chat: a compaction writes its
        // summary rows with the current time and leaves the `/compact` request
        // behind them carrying an older one (measured in a real session store).
        // Filtering every message against the anchor kept the late-but-older row
        // and dropped nothing before it — a branch with a hole in it.
        let conv = Conversation::new_unvalidated(vec![
            umsg(100, "q1"),
            amsg(101, "a1"),
            umsg(200, "summary"),
            amsg(201, "noted"),
            umsg(150, "/compact"), // stored after, created before
            amsg(202, "a2"),
            umsg(300, "q3"),
            amsg(301, "a3"),
        ]);

        // Legacy timestamp-only anchor at "noted" (201).
        let t = branched(&conv, None, Some(201));
        assert_eq!(t.messages().len(), 4);
        assert_eq!(t.messages().last().unwrap().as_concat_text(), "noted");
        assert!(
            t.messages()
                .iter()
                .all(|m| m.as_concat_text() != "/compact"),
            "a branch is a prefix; a later row with an older timestamp must not \
             be spliced in behind the cut"
        );
    }

    #[test]
    fn trim_by_timestamp_before_the_conversation_keeps_nothing() {
        let conv = Conversation::new_unvalidated(vec![umsg(100, "q1"), amsg(101, "a1")]);
        assert!(branched(&conv, None, Some(1)).messages().is_empty());
    }

    #[tokio::test]
    async fn diverge_with_a_stale_anchor_id_keeps_the_conversation_end_to_end() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        for message in tool_using_conversation().messages() {
            sm.add_message(&session.id, message).await.unwrap();
        }

        // The desktop clicked Branch on the last answer (created 304) but sent
        // the first reply's id alongside it.
        let branch = sm
            .diverge_session_at(
                &session.id,
                None,
                Some(304),
                Some("e1-preamble".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(branch.message_count, 15);
        let conversation = branch.conversation.unwrap();
        assert_eq!(
            conversation.messages().last().unwrap().as_concat_text(),
            "answer to q3"
        );
        // And the row must not claim a divergence point the branch was not cut
        // at: the refused anchor is never recorded.
        assert_eq!(branch.branch_point_msg_uid.as_deref(), Some("e3-answer"));
    }

    #[tokio::test]
    async fn diverge_records_the_anchor_it_actually_honoured() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        for message in tool_using_conversation().messages() {
            sm.add_message(&session.id, message).await.unwrap();
        }

        let branch = sm
            .diverge_session_at(&session.id, None, Some(204), Some("e2-answer".to_string()))
            .await
            .unwrap();
        assert_eq!(branch.message_count, 10);
        assert_eq!(branch.branch_point_msg_uid.as_deref(), Some("e2-answer"));

        // An unresolvable anchor records the message the branch really ends at.
        let branch = sm
            .diverge_session_at(&session.id, None, None, Some("never-stored".to_string()))
            .await
            .unwrap();
        assert_eq!(branch.message_count, 15);
        assert_eq!(branch.branch_point_msg_uid.as_deref(), Some("e3-answer"));
    }

    #[tokio::test]
    async fn test_diverge_trims_in_flight_turn_end_to_end() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // q0 → a0 (complete), then a follow-up question whose answer is still
        // being generated when the user hits Diverge.
        let original = seed_session_with_messages(&sm, 1).await;
        let now = chrono::Utc::now().timestamp_millis();
        sm.add_message(&original.id, &umsg(now + 10_000, "follow-up?"))
            .await
            .unwrap();

        let branch = sm.diverge_session(&original.id, None, None).await.unwrap();
        // The unanswered follow-up is NOT carried over; the branch ends at a0.
        assert_eq!(branch.message_count, 2);
        let last = branch
            .conversation
            .unwrap()
            .messages()
            .last()
            .unwrap()
            .as_concat_text();
        assert_eq!(last, "answer 0");
    }

    // ── BR-45: stable per-message ids + branch divergence point ──────────────

    /// Ids survive the exact operation that used to renumber them — a full
    /// history rewrite (compaction/edit). Every kept message keeps its id; only
    /// a newly-inserted message gets a fresh, non-positional id.
    #[tokio::test]
    async fn msg_uid_stable_across_replace_conversation() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = seed_session_with_messages(&sm, 2).await; // 4 messages
        let before: Vec<String> = session
            .conversation
            .as_ref()
            .unwrap()
            .messages()
            .iter()
            .map(|m| m.id.clone().unwrap())
            .collect();
        assert_eq!(before.len(), 4);
        // Durable ids are UUIDs, not the old positional `msg_{session}_{idx}`.
        assert!(before.iter().all(|id| !id.starts_with("msg_")));

        // Simulate a compaction: rewrite the same messages plus one brand-new
        // summary message that carries no id yet.
        let mut msgs: Vec<Message> = session.conversation.as_ref().unwrap().messages().to_vec();
        msgs.push(amsg(chrono::Utc::now().timestamp_millis() + 99, "summary"));
        sm.replace_conversation(&session.id, &Conversation::new_unvalidated(msgs))
            .await
            .unwrap();

        let after = sm.get_session(&session.id, true).await.unwrap();
        let after_ids: Vec<String> = after
            .conversation
            .unwrap()
            .messages()
            .iter()
            .map(|m| m.id.clone().unwrap())
            .collect();
        assert_eq!(after_ids.len(), 5);
        // The four kept messages preserved their ids across the rewrite.
        assert_eq!(&after_ids[..4], &before[..]);
        // The new summary got a fresh, distinct, non-positional id.
        let new_id = &after_ids[4];
        assert!(!new_id.starts_with("msg_"));
        assert!(!before.contains(new_id));
    }

    // ── conversation revision token ──────────────────────────────────────────

    async fn revision_session(sm: &SessionManager) -> String {
        sm.create_session(PathBuf::from("/tmp/rev"), "rev".into(), SessionType::User)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn conversation_revision_advances_on_append() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;

        let empty = sm.conversation_revision(&id).await.unwrap();
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        let one = sm.conversation_revision(&id).await.unwrap();
        sm.add_message(&id, &umsg(2, "two")).await.unwrap();
        let two = sm.conversation_revision(&id).await.unwrap();

        assert_ne!(empty, one);
        assert_ne!(one, two);
        assert_eq!(two.message_count(), 2);
    }

    /// The property a message COUNT cannot have: rewriting the same messages
    /// back changes the revision. This is why the freshness guard is not the
    /// length compare BR-12 shipped — an edit that drops one message plus the
    /// next turn's user message nets to zero and would pass a length check.
    #[tokio::test]
    async fn conversation_revision_advances_on_identical_rewrite() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &umsg(2, "two")).await.unwrap();

        let before = sm.conversation_revision(&id).await.unwrap();
        let same = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        sm.replace_conversation(&id, &same).await.unwrap();
        let after = sm.conversation_revision(&id).await.unwrap();

        assert_eq!(before.message_count(), after.message_count());
        assert_ne!(
            before, after,
            "an identical-content rewrite must still move the revision"
        );
    }

    /// AUTOINCREMENT never rewinds `sqlite_sequence`, so truncating and
    /// refilling to the same count cannot reproduce an earlier revision.
    #[tokio::test]
    async fn conversation_revision_never_ababs_across_truncate_and_refill() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &umsg(2, "two")).await.unwrap();
        let original = sm.conversation_revision(&id).await.unwrap();

        sm.truncate_conversation(&id, 2).await.unwrap();
        sm.add_message(&id, &umsg(3, "three")).await.unwrap();
        let refilled = sm.conversation_revision(&id).await.unwrap();

        assert_eq!(original.message_count(), refilled.message_count());
        assert_ne!(original, refilled, "revision must not ABA");
    }

    /// The mechanism the test below turns into data loss. An AUTOINCREMENT
    /// sequence exists precisely to keep a rowid non-reusable for the LIFETIME
    /// OF THE DATABASE; `DELETE FROM messages` already leaves it alone (that is
    /// what makes the truncate-and-refill test above hold). Deleting the
    /// `sqlite_sequence` row hands the next session the rowids of a session
    /// that is gone.
    #[tokio::test]
    async fn clearing_all_sessions_does_not_rewind_the_message_rowids() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let first = revision_session(&sm).await;
        sm.add_message(&first, &umsg(1, "one")).await.unwrap();
        sm.add_message(&first, &umsg(2, "two")).await.unwrap();
        let before = sm.conversation_revision(&first).await.unwrap();

        sm.clear_all_sessions().await.unwrap();

        let second = revision_session(&sm).await;
        sm.add_message(&second, &umsg(3, "three")).await.unwrap();
        let after = sm.conversation_revision(&second).await.unwrap();

        assert!(
            after.max_rowid > before.max_rowid,
            "a message written after the wipe must not reuse a rowid the wipe \
             freed ({} -> {})",
            before.max_rowid,
            after.max_rowid
        );
    }

    /// #51 W3: the revision must not ABA across a `/reset` History wipe either.
    ///
    /// `clear_all_sessions` empties every table, and `create_session` allocates
    /// `YYYYMMDD_N` as `MAX(suffix) + 1` over a now-empty `sessions` table — so
    /// the next session created the same day REUSES the id. If the message
    /// AUTOINCREMENT sequence is rewound with it, a rewrite still holding a
    /// revision from the previous incarnation finds its basis satisfied by a
    /// brand-new session's brand-new message and destroys it. Eager compaction
    /// is detached from its turn, so it really can still be in flight across a
    /// wipe.
    #[tokio::test]
    async fn a_guarded_rewrite_cannot_cross_a_clear_and_recreate() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let old = revision_session(&sm).await;
        sm.add_message(&old, &umsg(1, "first incarnation"))
            .await
            .unwrap();
        // A detached rewrite (eager compaction) snapshots the session...
        let (known, basis) = snapshot(&sm, &old).await;

        // ...and while it is still working, the user wipes History from /reset.
        sm.clear_all_sessions().await.unwrap();

        let new = revision_session(&sm).await;
        assert_eq!(new, old, "the ABA needs the session id to be reused");
        sm.add_message(&new, &umsg(2, "second incarnation"))
            .await
            .unwrap();

        let replacement =
            Conversation::new_unvalidated(vec![umsg(9, "summary of the first incarnation")]);
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&new, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            ReplaceOutcome::Stale,
            "a revision from a previous incarnation of this id must never match"
        );
        assert_eq!(
            stored_texts(&sm, &new).await,
            vec!["second incarnation".to_string()],
            "the new session's acknowledged message must survive the stale rewrite"
        );
    }

    /// ...and the incarnation token has to close it BY ITSELF, not merely as a
    /// consequence of leaving `sqlite_sequence` alone. Any number of things
    /// present a rewound sequence to a running process — a database restored
    /// from a backup, a hand-copied `sessions.db`, a future wipe path that
    /// re-creates the file. Here the pre-fix conditions are reproduced
    /// EXACTLY: the old session's rowids are replayed one for one, so
    /// `(count, max_rowid)` is byte-identical across the two incarnations and
    /// only the row identity can tell them apart.
    #[tokio::test]
    async fn a_guarded_rewrite_is_refused_even_when_the_rowids_are_replayed_exactly() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let old = revision_session(&sm).await;
        sm.add_message(&old, &umsg(1, "first incarnation"))
            .await
            .unwrap();
        let (known, basis) = snapshot(&sm, &old).await;

        sm.clear_all_sessions().await.unwrap();
        // Rewind the message sequence behind the store's back.
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query("DELETE FROM sqlite_sequence WHERE name = 'messages'")
            .execute(pool)
            .await
            .unwrap();

        let new = revision_session(&sm).await;
        assert_eq!(new, old);
        sm.add_message(&new, &umsg(2, "second incarnation"))
            .await
            .unwrap();

        let replayed = sm.conversation_revision(&new).await.unwrap();
        assert_eq!(
            (replayed.count, replayed.max_rowid),
            (basis.count, basis.max_rowid),
            "this test is only meaningful if the rowids really were replayed"
        );

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&new, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::Stale);
        assert_eq!(
            stored_texts(&sm, &new).await,
            vec!["second incarnation".to_string()]
        );
    }

    #[tokio::test]
    async fn conversation_revision_is_zero_for_an_empty_session() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        let revision = sm.conversation_revision(&id).await.unwrap();
        assert_eq!(revision.count, 0);
        assert_eq!(revision.max_rowid, 0);
    }

    /// A rewrite changes the session's content, so it must move `updated_at`
    /// like any other write — otherwise a compacted or edited session sorts as
    /// untouched in the `ORDER BY updated_at DESC` session list. (The bump is
    /// also the rewrite transaction's write-first lock acquisition; see the
    /// comment on `replace_conversation_inner`.)
    #[tokio::test]
    async fn a_conversation_rewrite_bumps_the_session_updated_at() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let s = sm
            .create_session(PathBuf::from("/tmp/a"), "s".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(&s.id, &umsg(1, "hello")).await.unwrap();

        // Back-date it so the one-second resolution of `datetime('now')` cannot
        // make a real bump look like no change.
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query("UPDATE sessions SET updated_at = '2000-01-01 00:00:00' WHERE id = ?")
            .bind(&s.id)
            .execute(pool)
            .await
            .unwrap();
        let before = sm.get_session(&s.id, false).await.unwrap().updated_at;

        sm.replace_conversation(&s.id, &Conversation::new_unvalidated(vec![umsg(2, "bye")]))
            .await
            .unwrap();

        let after = sm.get_session(&s.id, false).await.unwrap().updated_at;
        assert!(
            after > before,
            "a whole-history rewrite must bump updated_at ({before} -> {after})"
        );
    }

    /// ...but an IMPORT is the one caller that must not inherit that bump.
    ///
    /// `import_legacy_session` INSERTs the historical `created_at`/`updated_at`
    /// and then writes the conversation through `replace_conversation_inner`,
    /// whose write-first statement stamps `updated_at = datetime('now')` and
    /// beats the back-dated value (the test above is what proves it beats it).
    /// Every legacy JSONL session would then sort as "just now", collapsing a
    /// user's whole history to today in the sidebar on the single migration run
    /// that imports it.
    #[tokio::test]
    async fn importing_a_legacy_session_keeps_its_historical_updated_at() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        // Any call opens (and migrates) the database.
        sm.create_session(PathBuf::from("/tmp/live"), "live".into(), SessionType::User)
            .await
            .unwrap();
        let pool = sm.storage().pool().await.unwrap();

        let historical: DateTime<Utc> = "2021-03-04T05:06:07Z".parse().unwrap();
        let legacy = Session {
            id: "legacy-1".into(),
            working_dir: PathBuf::from("/tmp/legacy"),
            name: "an old chat".into(),
            user_set_name: false,
            session_type: SessionType::User,
            created_at: historical,
            updated_at: historical,
            extension_data: Default::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: None,
            workflow: None,
            user_workflow_values: None,
            conversation: Some(Conversation::new_unvalidated(vec![umsg(1, "hi")])),
            message_count: 1,
            provider_name: None,
            model_config: None,
            diverged_from: None,
            branch_point_msg_uid: None,
            parent_session_id: None,
            privacy_tier: SessionClassification::Public,
            privacy_reason: None,
        };
        SessionStorage::import_legacy_session(pool, &legacy)
            .await
            .unwrap();

        let imported = sm.get_session("legacy-1", true).await.unwrap();
        assert_eq!(
            imported.updated_at, historical,
            "an imported session must keep its historical mtime, not the \
             import's wall clock"
        );
        assert_eq!(imported.created_at, historical);
        // The conversation itself still landed.
        assert_eq!(imported.conversation.unwrap().messages().len(), 1);
    }

    // ── replace_conversation_preserving_tail ─────────────────────────────────

    /// Snapshot a session the way every rewrite caller must.
    async fn snapshot(sm: &SessionManager, id: &str) -> (Conversation, ConversationRevision) {
        let (session, revision) = sm.snapshot_for_rewrite(id).await.unwrap();
        (session.conversation.unwrap(), revision)
    }

    async fn stored_texts(sm: &SessionManager, id: &str) -> Vec<String> {
        sm.get_session(id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The no-concurrency path must be exactly the old behaviour.
    #[tokio::test]
    async fn preserving_tail_writes_verbatim_when_nothing_moved() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &amsg(2, "two")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;
        let replacement = Conversation::new_unvalidated(vec![known.messages()[1].clone()]);

        let (outcome, stored) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::Replaced);
        assert_eq!(stored.messages().len(), 1);
        assert_eq!(stored_texts(&sm, &id).await, vec!["two".to_string()]);
    }

    /// THE headline case: a message appended while the caller was computing its
    /// rewrite must survive. This is BR-71's `mode: "note"`, and the shipped
    /// `biorouter term log` cross-process append.
    #[tokio::test]
    async fn preserving_tail_carries_over_a_concurrent_append() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &amsg(2, "two")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;

        // ...another writer appends while the "summarizer" runs.
        let note_uid = sm
            .add_message(&id, &umsg(3, "NOTE from elsewhere"))
            .await
            .unwrap();

        // ...and the caller writes back a compaction of what it saw.
        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, stored) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            ReplaceOutcome::ReplacedPreservingTail { preserved: 1 }
        );

        let texts = stored_texts(&sm, &id).await;
        assert_eq!(
            texts,
            vec!["summary".to_string(), "NOTE from elsewhere".to_string()],
            "the note must survive, after the compacted head"
        );
        // The returned conversation agrees with the store...
        assert_eq!(
            stored
                .messages()
                .iter()
                .filter_map(|m| m.id.clone())
                .collect::<Vec<_>>()
                .last()
                .cloned(),
            Some(note_uid.clone())
        );
        // ...and the note kept its stable id across the rewrite.
        let reloaded = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(
            reloaded.messages()[1].id.as_deref(),
            Some(note_uid.as_str())
        );
    }

    /// The discriminator against a watermark-only implementation: the writer's
    /// OWN messages, appended after it captured its basis and then deliberately
    /// compacted away, must stay gone.
    #[tokio::test]
    async fn preserving_tail_does_not_resurrect_the_writers_own_compacted_messages() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();

        let (_, basis) = snapshot(&sm, &id).await;
        // The caller appends its own messages past the watermark...
        sm.add_message(&id, &amsg(2, "mine-a")).await.unwrap();
        sm.add_message(&id, &amsg(3, "mine-b")).await.unwrap();
        // ...so its view (taken now) contains them.
        let known = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::Replaced);
        assert_eq!(stored_texts(&sm, &id).await, vec!["summary".to_string()]);
    }

    /// The mirror discriminator, against a uid-set-only implementation: a
    /// message the snapshot saw and the compaction dropped must stay dropped.
    #[tokio::test]
    async fn preserving_tail_does_not_resurrect_messages_the_snapshot_already_dropped() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "drop me")).await.unwrap();
        sm.add_message(&id, &amsg(2, "keep me")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;
        let replacement = Conversation::new_unvalidated(vec![known.messages()[1].clone()]);

        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::Replaced);
        assert_eq!(stored_texts(&sm, &id).await, vec!["keep me".to_string()]);
    }

    #[tokio::test]
    async fn preserving_tail_reports_stale_after_a_concurrent_truncate() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &amsg(2, "two")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;
        // A checkpoint restore / message edit removes the tail underneath us.
        sm.truncate_conversation(&id, 2).await.unwrap();
        let before = sm.conversation_revision(&id).await.unwrap();

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, returned) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::Stale);
        assert_eq!(
            sm.conversation_revision(&id).await.unwrap(),
            before,
            "a stale rewrite must not write anything at all"
        );
        assert_eq!(stored_texts(&sm, &id).await, vec!["one".to_string()]);
        assert_eq!(returned.messages().len(), 1, "the replacement, unchanged");
    }

    #[tokio::test]
    async fn preserving_tail_reports_stale_after_a_concurrent_wholesale_rewrite() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;
        // Another rewrite lands first: every row is renumbered above the
        // watermark, so the prefix count goes to 0.
        sm.replace_conversation(&id, &Conversation::new_unvalidated(vec![umsg(5, "theirs")]))
            .await
            .unwrap();

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::Stale);
        assert_eq!(stored_texts(&sm, &id).await, vec!["theirs".to_string()]);
    }

    #[tokio::test]
    async fn preserving_tail_reports_session_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        // Force the pool + schema to exist.
        let _ = revision_session(&sm).await;

        let empty = Conversation::default();
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(
                "no-such-session",
                &Conversation::new_unvalidated(vec![umsg(1, "x")]),
                ConversationRevision::from_parts(0, 0, 0),
                &empty,
            )
            .await
            .unwrap();

        assert_eq!(outcome, ReplaceOutcome::SessionNotFound);
        assert_eq!(
            sm.conversation_revision("no-such-session")
                .await
                .unwrap()
                .message_count(),
            0,
            "nothing may be written for a session that does not exist"
        );
    }

    #[tokio::test]
    async fn preserving_tail_orders_recovered_messages_last_by_rowid() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;
        // Deliberately out-of-order `created` values: insertion order, not the
        // timestamp, is what the read path (`ORDER BY id`) reproduces.
        sm.add_message(&id, &umsg(50, "note-a")).await.unwrap();
        sm.add_message(&id, &umsg(40, "note-b")).await.unwrap();

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, _) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            ReplaceOutcome::ReplacedPreservingTail { preserved: 2 }
        );
        assert_eq!(
            stored_texts(&sm, &id).await,
            vec![
                "summary".to_string(),
                "note-a".to_string(),
                "note-b".to_string()
            ]
        );
    }

    /// A row a schema upgrade has not backfilled has a NULL `msg_uid`, and the
    /// read path synthesizes `msg_{session}_{idx}` for it. The recovery scan
    /// must synthesize the SAME id, or such a row would look foreign forever
    /// and be duplicated on every rewrite.
    #[tokio::test]
    async fn preserving_tail_recovers_a_legacy_null_msg_uid_row() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();

        let (known, basis) = snapshot(&sm, &id).await;

        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json, msg_uid) \
             VALUES (?, 'user', ?, 7, NULL, NULL)",
        )
        .bind(&id)
        .bind(serde_json::to_string(&vec![MessageContent::text("legacy note")]).unwrap())
        .execute(pool)
        .await
        .unwrap();

        // The read path's synthesized id for that row (index 1 of the session).
        let read_id = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()[1]
            .id
            .clone()
            .unwrap();
        assert_eq!(read_id, format!("msg_{id}_1"));

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "summary")]);
        let (outcome, stored) = sm
            .replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            ReplaceOutcome::ReplacedPreservingTail { preserved: 1 }
        );
        assert_eq!(
            stored.messages()[1].id.as_deref(),
            Some(read_id.as_str()),
            "the recovery scan must reproduce the read path's synthesized id"
        );
        assert_eq!(
            stored_texts(&sm, &id).await,
            vec!["summary".to_string(), "legacy note".to_string()]
        );
    }

    /// A recovered message must be findable by chat recall — the FTS mirror is
    /// rebuilt from the merged list, not from the caller's replacement.
    #[tokio::test]
    async fn preserving_tail_indexes_recovered_messages_for_chat_recall() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "photosynthesis"))
            .await
            .unwrap();

        let (known, basis) = snapshot(&sm, &id).await;
        sm.add_message(&id, &umsg(2, "chemiosmosis in mitochondria"))
            .await
            .unwrap();

        let replacement = Conversation::new_unvalidated(vec![umsg(9, "a compaction summary")]);
        sm.replace_conversation_preserving_tail(&id, &replacement, basis, &known)
            .await
            .unwrap();

        assert_eq!(
            sm.search_chat_history(
                "chemiosmosis",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap()
            .results
            .len(),
            1,
            "a recovered message must be indexed for recall"
        );
        assert!(
            sm.search_chat_history(
                "photosynthesis",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap()
            .results
            .is_empty(),
            "a compacted-away message must drop out of the index"
        );
    }

    // ── truncate_conversation ────────────────────────────────────────────────

    async fn fts_hits(sm: &SessionManager, term: &str) -> usize {
        sm.search_chat_history(
            term,
            None,
            None,
            None,
            None,
            crate::session::chat_history_search::SearchReach::tier_only(
                crate::privacy::ProviderTier::Private,
            ),
        )
        .await
        .unwrap()
        .results
        .len()
    }

    /// Rows in the recall mirror with no message behind them.
    async fn orphan_fts_rows(sm: &SessionManager, id: &str) -> i64 {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM messages_fts f WHERE f.session_id = ? \
             AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.id = f.message_id)",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// #51 W4: a checkpoint restore / message edit deletes message rows without
    /// touching the FTS recall mirror, so the mirror keeps a row per dropped
    /// message forever — every restore leaks more, and the only thing keeping
    /// them from surfacing as hits is an `INNER JOIN messages` in an unrelated
    /// query. The rewrite path has always kept the two in lockstep; truncation
    /// never did.
    #[tokio::test]
    async fn truncating_a_conversation_drops_its_rows_from_chat_recall() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "photosynthesis"))
            .await
            .unwrap();
        sm.add_message(&id, &amsg(2, "chemiosmosis in mitochondria"))
            .await
            .unwrap();
        assert_eq!(fts_hits(&sm, "chemiosmosis").await, 1, "seeded");

        sm.truncate_conversation(&id, 2).await.unwrap();

        assert_eq!(
            orphan_fts_rows(&sm, &id).await,
            0,
            "the recall mirror must not keep a row for a message that is gone"
        );
        assert_eq!(
            fts_hits(&sm, "chemiosmosis").await,
            0,
            "a truncated message must drop out of chat recall"
        );
        assert_eq!(
            fts_hits(&sm, "photosynthesis").await,
            1,
            "...and a surviving one must stay in it"
        );
    }

    /// #51 W4: truncation changes the session's content, so it must move
    /// `updated_at` for the same reason a rewrite must — otherwise an edited or
    /// restored session sorts as untouched in the `ORDER BY updated_at DESC`
    /// session list.
    #[tokio::test]
    async fn truncating_a_conversation_bumps_the_session_updated_at() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &amsg(2, "two")).await.unwrap();

        let pool = sm.storage().pool().await.unwrap();
        sqlx::query("UPDATE sessions SET updated_at = '2000-01-01 00:00:00' WHERE id = ?")
            .bind(&id)
            .execute(pool)
            .await
            .unwrap();
        let before = sm.get_session(&id, false).await.unwrap().updated_at;

        sm.truncate_conversation(&id, 2).await.unwrap();

        let after = sm.get_session(&id, false).await.unwrap().updated_at;
        assert!(
            after > before,
            "a truncation must bump updated_at ({before} -> {after})"
        );
    }

    /// #51 W4: the deletion range is `created_timestamp >= ts`, which is open
    /// above — so a message appended after the caller decided where to cut is
    /// necessarily inside it and is destroyed, after its writer was told the
    /// append succeeded. Bounding by the rowid watermark the caller's view
    /// actually covered is what separates "the tail the user asked to drop"
    /// from "a message that arrived while we were deciding".
    #[tokio::test]
    async fn a_bounded_truncate_keeps_an_append_the_caller_never_saw() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = revision_session(&sm).await;
        sm.add_message(&id, &umsg(1, "one")).await.unwrap();
        sm.add_message(&id, &amsg(2, "two")).await.unwrap();

        // The caller reads the conversation and decides to cut at ts 2.
        let basis = sm.conversation_revision(&id).await.unwrap();

        // ...and another writer appends before the delete lands. Its timestamp
        // is necessarily >= the cut.
        sm.add_message(&id, &umsg(9, "NOTE from elsewhere"))
            .await
            .unwrap();

        assert_eq!(
            sm.truncate_conversation_bounded(&id, 2, basis)
                .await
                .unwrap(),
            TruncateOutcome::Truncated { removed: 1 }
        );

        assert_eq!(
            stored_texts(&sm, &id).await,
            vec!["one".to_string(), "NOTE from elsewhere".to_string()],
            "the cut tail goes; the append the caller never saw stays"
        );
    }

    /// A watermark from a previous incarnation of the id describes rowids that
    /// belonged to a different conversation, so it may not bound a delete in
    /// this one — the same reasoning as the guarded rewrite (#51 W3).
    #[tokio::test]
    async fn a_bounded_truncate_refuses_a_basis_from_a_previous_incarnation() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let old = revision_session(&sm).await;
        sm.add_message(&old, &umsg(1, "first incarnation"))
            .await
            .unwrap();
        let basis = sm.conversation_revision(&old).await.unwrap();

        sm.clear_all_sessions().await.unwrap();
        let new = revision_session(&sm).await;
        assert_eq!(new, old);
        sm.add_message(&new, &umsg(2, "second incarnation"))
            .await
            .unwrap();

        assert_eq!(
            sm.truncate_conversation_bounded(&new, 1, basis)
                .await
                .unwrap(),
            TruncateOutcome::Stale
        );
        assert_eq!(
            stored_texts(&sm, &new).await,
            vec!["second incarnation".to_string()],
            "a refused truncation must not delete anything"
        );
    }

    #[tokio::test]
    async fn a_bounded_truncate_reports_a_missing_session() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let _ = revision_session(&sm).await;
        let basis = sm.conversation_revision("no-such-session").await.unwrap();

        assert_eq!(
            sm.truncate_conversation_bounded("no-such-session", 0, basis)
                .await
                .unwrap(),
            TruncateOutcome::SessionNotFound
        );
    }

    /// Real overlap against a real pool and a real WAL file: 200 appends racing
    /// 60 snapshot -> "summarize" -> write-back cycles.
    ///
    /// Assertion 2 is what makes this irreplaceable. If the write-first
    /// `UPDATE sessions` at the top of the rewrite transaction is ever
    /// "simplified away" as a gratuitous write, the deferred transaction reads
    /// before it writes and the DELETE fails SQLITE_BUSY_SNAPSHOT *instantly*,
    /// bypassing the busy timeout. That surfaces here as an error, not as data
    /// loss, so a test that only checked the final message set would pass while
    /// the fix was broken.
    ///
    /// A busy *append* is a different animal from a busy *rewrite* and is
    /// tolerated here, exactly as `conversation_writeback_stress.rs` does. This
    /// workload appends with only a `yield_now` between writes against a
    /// rewriter looping on a 2 ms gap, so the rewriter re-takes the single write
    /// lock faster than a starved appender's 5 s `busy_timeout` can expire.
    /// That contention is pre-existing (it reproduces on the unguarded
    /// `replace_conversation` path), it is loud — `add_message` returns `Err`,
    /// so the caller knows — and it vanishes at realistic compaction gaps. See
    /// "What it costs" in `docs/agent-loop/conversation-writeback-freshness.md`.
    /// The invariant is unweakened: only ACKNOWLEDGED appends enter `uids`, and
    /// every one of those must still be on disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_appends_survive_racing_rewrites() {
        let temp = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let session = sm
            .create_session(PathBuf::from("/tmp"), "race".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(&session.id, &umsg(1, "seed")).await.unwrap();

        let appender = {
            let sm = Arc::clone(&sm);
            let id = session.id.clone();
            tokio::spawn(async move {
                let mut uids = Vec::new();
                let mut busy = 0usize;
                for i in 0..200 {
                    let m = umsg(100 + i, &format!("note-{i}"));
                    match sm.add_message(&id, &m).await {
                        Ok(uid) => uids.push(uid),
                        Err(e) if e.to_string().contains("database is locked") => busy += 1,
                        Err(e) => panic!(
                            "append failed for a reason other than lock \
                                          contention: {e}"
                        ),
                    }
                    tokio::task::yield_now().await;
                }
                (uids, busy)
            })
        };
        let rewriter = {
            let sm = Arc::clone(&sm);
            let id = session.id.clone();
            tokio::spawn(async move {
                let mut outcomes = Vec::new();
                let mut errors = Vec::new();
                for i in 0..60 {
                    let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
                    let known = session.conversation.unwrap();
                    // Stand-in for the summarization round-trip.
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    let mut msgs = known.messages().clone();
                    msgs.push(amsg(1_000 + i, &format!("summary-{i}")));
                    match sm
                        .replace_conversation_preserving_tail(
                            &id,
                            &Conversation::new_unvalidated(msgs),
                            basis,
                            &known,
                        )
                        .await
                    {
                        Ok((outcome, _)) => outcomes.push(outcome),
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                (outcomes, errors)
            })
        };
        let (uids, rewrites) = tokio::join!(appender, rewriter);
        let (uids, busy_appends) = uids.unwrap();
        let (outcomes, errors) = rewrites.unwrap();

        // 2. The write-first lock ordering held. A busy REWRITE is always a
        // hard failure: it means the transaction read before it wrote.
        assert!(
            errors.is_empty(),
            "no rewrite may fail (a `database is locked` here means the \
             transaction read before it wrote): {errors:?}"
        );
        // The race must not have degenerated into "the appender never got in",
        // which would make every assertion below vacuous.
        assert!(
            uids.len() >= 150,
            "only {} of 200 appends were acknowledged ({busy_appends} lost the \
             write lock): too few for this to still be testing anything",
            uids.len()
        );
        // 4. Every rewrite reported a real, non-silent outcome.
        assert_eq!(outcomes.len(), 60);

        let stored = sm
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let stored_ids: Vec<String> = stored
            .messages()
            .iter()
            .filter_map(|m| m.id.clone())
            .collect();

        // 1. Nothing appended was destroyed...
        let lost: Vec<&String> = uids.iter().filter(|u| !stored_ids.contains(u)).collect();
        assert!(
            lost.is_empty(),
            "{} of {} appended messages were destroyed by racing rewrites",
            lost.len(),
            uids.len()
        );
        // 3. ...and nothing was duplicated (UNIQUE(session_id, msg_uid) held).
        let mut unique = stored_ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), stored_ids.len(), "duplicate msg_uid stored");
    }

    /// The CLI-vs-daemon shape: two INDEPENDENT stores (separate connection
    /// pools, no shared in-memory state) over one `sessions.db`, exactly as a
    /// terminal `biorouter` and the desktop `biorouterd` see it. Nothing in
    /// process memory orders these — only the guard inside the rewrite
    /// transaction does.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn appends_from_a_second_store_survive_racing_rewrites() {
        let temp = TempDir::new().unwrap();
        let daemon = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let session = daemon
            .create_session(PathBuf::from("/tmp"), "cross".into(), SessionType::User)
            .await
            .unwrap();
        daemon
            .add_message(&session.id, &umsg(1, "seed"))
            .await
            .unwrap();

        // A second store over the same file — its own pool, its own WAL reader.
        let cli = Arc::new(SessionManager::new(temp.path().to_path_buf()));

        let appender = {
            let cli = Arc::clone(&cli);
            let id = session.id.clone();
            tokio::spawn(async move {
                // Same busy-append tolerance as
                // `concurrent_appends_survive_racing_rewrites`, and for the same
                // reason — only acknowledged appends are held to the invariant.
                let mut uids = Vec::new();
                let mut busy = 0usize;
                for i in 0..80 {
                    let m = umsg(100 + i, &format!("term-log-{i}"));
                    match cli.add_message(&id, &m).await {
                        Ok(uid) => uids.push(uid),
                        Err(e) if e.to_string().contains("database is locked") => busy += 1,
                        Err(e) => panic!(
                            "cross-pool append failed for a reason other \
                                          than lock contention: {e}"
                        ),
                    }
                    tokio::task::yield_now().await;
                }
                (uids, busy)
            })
        };
        let rewriter = {
            let daemon = Arc::clone(&daemon);
            let id = session.id.clone();
            tokio::spawn(async move {
                let mut errors = Vec::new();
                for i in 0..25 {
                    let (session, basis) = daemon.snapshot_for_rewrite(&id).await.unwrap();
                    let known = session.conversation.unwrap();
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    let mut msgs = known.messages().clone();
                    msgs.push(amsg(1_000 + i, &format!("summary-{i}")));
                    if let Err(e) = daemon
                        .replace_conversation_preserving_tail(
                            &id,
                            &Conversation::new_unvalidated(msgs),
                            basis,
                            &known,
                        )
                        .await
                    {
                        errors.push(e.to_string());
                    }
                }
                errors
            })
        };
        let (uids, errors) = tokio::join!(appender, rewriter);
        let (uids, busy_appends) = uids.unwrap();
        let errors = errors.unwrap();

        assert!(
            errors.is_empty(),
            "cross-pool rewrites must not fail: {errors:?}"
        );
        assert!(
            uids.len() >= 60,
            "only {} of 80 cross-pool appends were acknowledged \
             ({busy_appends} lost the write lock): too few to test anything",
            uids.len()
        );

        let stored_ids: Vec<String> = daemon
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .filter_map(|m| m.id.clone())
            .collect();
        let lost: Vec<&String> = uids.iter().filter(|u| !stored_ids.contains(u)).collect();
        assert!(
            lost.is_empty(),
            "{} of {} messages appended by the other store were destroyed",
            lost.len(),
            uids.len()
        );
    }

    /// Two racing preserving-tail rewrites must not collide on
    /// UNIQUE(session_id, msg_uid): the rewrite path has no duplicate-uid
    /// recovery of its own, unlike `add_message`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_rewrites_do_not_produce_duplicate_msg_uids() {
        let temp = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let session = sm
            .create_session(PathBuf::from("/tmp"), "race2".into(), SessionType::User)
            .await
            .unwrap();
        for i in 0..5 {
            sm.add_message(&session.id, &umsg(i, &format!("m{i}")))
                .await
                .unwrap();
        }

        let spawn_rewriter = |tag: &'static str| {
            let sm = Arc::clone(&sm);
            let id = session.id.clone();
            tokio::spawn(async move {
                let mut errors = Vec::new();
                for i in 0..40 {
                    let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
                    let known = session.conversation.unwrap();
                    tokio::task::yield_now().await;
                    let mut msgs = known.messages().clone();
                    msgs.push(amsg(2_000 + i, &format!("{tag}-{i}")));
                    if let Err(e) = sm
                        .replace_conversation_preserving_tail(
                            &id,
                            &Conversation::new_unvalidated(msgs),
                            basis,
                            &known,
                        )
                        .await
                    {
                        errors.push(e.to_string());
                    }
                }
                errors
            })
        };
        let (a, b) = tokio::join!(spawn_rewriter("a"), spawn_rewriter("b"));
        let mut errors = a.unwrap();
        errors.extend(b.unwrap());
        assert!(
            errors.is_empty(),
            "racing rewrites must not error: {errors:?}"
        );

        let stored_ids: Vec<String> = sm
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .filter_map(|m| m.id.clone())
            .collect();
        let mut unique = stored_ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), stored_ids.len(), "duplicate msg_uid stored");
    }

    /// #41 resilience: inserting a message whose caller-supplied id already
    /// exists in the session with DIFFERENT content must NOT abort — the store
    /// re-mints the uid, keeps both rows, and RETURNS the effective uid so the
    /// caller's in-memory message can adopt it. Before this, the duplicate hit
    /// `UNIQUE(session_id, msg_uid)` (SQLite 2067) and the whole turn died;
    /// then the re-mint happened only in SQLite while the caller kept the
    /// stale id.
    #[tokio::test]
    async fn add_message_reminting_recovers_from_a_duplicate_uid() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/dup_uid"),
                "Dup uid".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis();
        let first_uid = sm
            .add_message(&session.id, &amsg(now, "first").with_id("shared-uid"))
            .await
            .unwrap();
        assert_eq!(
            first_uid, "shared-uid",
            "the happy path returns the caller-supplied uid"
        );
        // The forced duplicate: same session, same caller-supplied id,
        // different content.
        let reminted_uid = sm
            .add_message(&session.id, &amsg(now + 1, "second").with_id("shared-uid"))
            .await
            .expect("a duplicate uid must be re-minted, not abort the turn");
        assert_ne!(
            reminted_uid, "shared-uid",
            "the returned uid must be the re-minted one, not the stale caller id"
        );

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        let messages = loaded.conversation.unwrap();
        assert_eq!(messages.len(), 2, "both messages must be persisted");
        let ids: Vec<String> = messages
            .messages()
            .iter()
            .map(|m| m.id.clone().unwrap())
            .collect();
        assert_eq!(ids[0], "shared-uid", "the first insert keeps its id");
        assert_eq!(
            ids[1], reminted_uid,
            "the persisted row and the returned uid must agree"
        );
        let texts: Vec<String> = messages
            .messages()
            .iter()
            .map(Message::as_concat_text)
            .collect();
        assert_eq!(texts, vec!["first", "second"]);

        // An id duplicated across DIFFERENT sessions is fine and untouched.
        let other = sm
            .create_session(
                PathBuf::from("/tmp/dup_uid2"),
                "Other".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(&other.id, &amsg(now + 2, "elsewhere").with_id("shared-uid"))
            .await
            .unwrap();
        let other_loaded = sm.get_session(&other.id, true).await.unwrap();
        assert_eq!(
            other_loaded.conversation.unwrap().messages()[0]
                .id
                .as_deref(),
            Some("shared-uid")
        );
    }

    /// #41 idempotence: re-adding the EXACT same message (same uid, identical
    /// role/content/metadata — a caller retrying a write it believes failed)
    /// is success, not a re-mint. One row, same uid back, no duplicate.
    #[tokio::test]
    async fn add_message_treats_an_exact_replay_as_success() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/replay_uid"),
                "Replay".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis();
        let message = amsg(now, "same words").with_id("replay-uid");
        let first = sm.add_message(&session.id, &message).await.unwrap();
        let second = sm
            .add_message(&session.id, &message)
            .await
            .expect("an exact replay must be idempotent success");
        assert_eq!(first, "replay-uid");
        assert_eq!(
            second, "replay-uid",
            "the replay returns the SAME uid, so the caller's in-memory id \
             stays in agreement"
        );

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        let messages = loaded.conversation.unwrap();
        assert_eq!(
            messages.len(),
            1,
            "an exact replay must not create a duplicate row"
        );
        assert_eq!(messages.messages()[0].id.as_deref(), Some("replay-uid"));
    }

    /// #41 caller contract: after adopting the re-minted uid returned by a
    /// collision, re-persisting that same in-memory message is an exact
    /// replay — success, no third row. This is the loop the agent's persist
    /// batch runs (adopt effective uid → later replays are idempotent).
    #[tokio::test]
    async fn adopted_reminted_uid_makes_later_replays_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/adopt_uid"),
                "Adopt".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis();
        sm.add_message(&session.id, &amsg(now, "original").with_id("clash-uid"))
            .await
            .unwrap();

        // Collision with different content: the store re-mints and the caller
        // adopts the returned uid (what agent.rs does after add_message).
        let mut colliding = amsg(now + 1, "different").with_id("clash-uid");
        let effective = sm.add_message(&session.id, &colliding).await.unwrap();
        assert_ne!(effective, "clash-uid");
        colliding.id = Some(effective.clone());

        // Replaying the adopted message is idempotent success.
        let replay = sm.add_message(&session.id, &colliding).await.unwrap();
        assert_eq!(replay, effective);

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        let messages = loaded.conversation.unwrap();
        assert_eq!(messages.len(), 2, "adopt-then-replay must not add a row");
        let ids: Vec<&str> = messages
            .messages()
            .iter()
            .map(|m| m.id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, vec!["clash-uid", effective.as_str()]);
    }

    /// #41: a message with NO caller id gets a minted uid, and the caller is
    /// told which one, so it can stamp its in-memory copy.
    #[tokio::test]
    async fn add_message_returns_the_minted_uid_for_idless_messages() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/minted_uid"),
                "Minted".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis();
        let minted = sm
            .add_message(&session.id, &amsg(now, "no id"))
            .await
            .unwrap();
        assert!(!minted.is_empty());

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        assert_eq!(
            loaded.conversation.unwrap().messages()[0].id.as_deref(),
            Some(minted.as_str()),
            "the persisted uid and the returned uid must agree"
        );
    }

    /// Deleting an unknown id must report "Session not found" and touch nothing.
    ///
    /// This pins the contract across a change in HOW it is produced. The delete
    /// used to open its transaction with `SELECT EXISTS` and bail before
    /// touching anything; it now issues the DELETEs first and decides from
    /// `rows_affected`, because a leading SELECT pins a WAL read snapshot and
    /// the later write has to upgrade — which SQLite refuses with
    /// SQLITE_BUSY_SNAPSHOT without consulting the busy handler, so the pool's
    /// five-second timeout could not save it.
    ///
    /// What this test does and does not prove, stated plainly: for an unknown
    /// id every DELETE matches zero rows, so commit and rollback are
    /// indistinguishable and this does NOT exercise the rollback. It pins the
    /// externally visible contract — the same error, and no collateral damage
    /// to a sibling session or its messages. The rollback is only observable
    /// against orphaned message rows whose session row is already gone, which
    /// this store has no supported way to create.
    #[tokio::test]
    async fn deleting_an_unknown_id_rolls_back_and_leaves_real_sessions_intact() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let keep = sm
            .create_session(
                PathBuf::from("/tmp/delete_rollback"),
                "KeepMe".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(
            &keep.id,
            &amsg(chrono::Utc::now().timestamp_millis(), "hello"),
        )
        .await
        .unwrap();

        let err = sm
            .delete_session("no-such-session-id")
            .await
            .expect_err("deleting an id that does not exist must fail");
        assert!(
            err.to_string().contains("Session not found"),
            "unexpected error: {err}"
        );

        // The real session and its message must both survive the rolled-back
        // transaction — this is the assertion the change actually needs.
        let loaded = sm
            .get_session(&keep.id, true)
            .await
            .expect("the untouched session must still exist");
        assert_eq!(
            loaded.conversation.unwrap().messages().len(),
            1,
            "a failed delete of a different id must not remove messages"
        );
    }

    /// Real overlap against a real pool and a real WAL file: 150 deletes of
    /// real sessions racing a second, independent store that never stops
    /// committing.
    ///
    /// This is the delete-side twin of
    /// `concurrent_appends_survive_racing_rewrites`, and `errors.is_empty()` is
    /// what makes it irreplaceable. `delete_session` opens its DEFERRED
    /// transaction with `DELETE FROM messages` deliberately. Restore the
    /// `SELECT EXISTS` probe it used to lead with — the tidier, obvious
    /// "check, then act" — and the transaction pins a WAL read snapshot before
    /// it writes; the very next statement has to *upgrade* to a writer, and any
    /// commit that landed in that window makes SQLite refuse **instantly** with
    /// SQLITE_BUSY_SNAPSHOT. A busy handler is not consulted for that error, so
    /// the pool's five-second `busy_timeout` is not in the path at all, and the
    /// delete surfaces `(code: 5) database is locked` when it should merely
    /// have waited its turn.
    ///
    /// The damage is an ERROR, not data loss — the refused transaction rolls
    /// back and leaves the session exactly as it was. So a test that only
    /// checked which sessions survived would pass while the ordering was
    /// broken, which is precisely why
    /// `deleting_an_unknown_id_rolls_back_and_leaves_real_sessions_intact`
    /// cannot stand in for this one: it is single-threaded, on one connection,
    /// with nothing to race.
    ///
    /// Mutation-tested, not assumed. Restoring the leading `SELECT EXISTS` and
    /// changing nothing else failed 16 of 16 runs, 25 to 71 deletes lost per
    /// run; removing it again passed 26 of 26. The reported codes are
    /// `(code: 5) database is locked` and, less often, `(code: 517)` — 517 *is*
    /// SQLITE_BUSY_SNAPSHOT (`SQLITE_BUSY | (2<<8)`), and both are the same
    /// root cause. SQLite skips the busy handler for either once
    /// `pBt->inTransaction` is already `TRANS_READ`, so a delete that has read
    /// first cannot wait for the lock at all: 5 when the writer merely *holds*
    /// it, 517 when the writer has *committed* since the snapshot.
    ///
    /// A busy *append* on the writer side is tolerated, exactly as the two
    /// rewrite races above tolerate it and for the same reason: the writer
    /// loops with only a `yield_now`, so it can lose the single per-file write
    /// lock to a delete that is legitimately holding it. Only the DELETE side
    /// is held to "never busy".
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_deletes_survive_racing_writes() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        const DOOMED: usize = 150;

        let temp = TempDir::new().unwrap();
        let deleter_store = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        // A second store over the same `sessions.db`: its own pool, its own WAL
        // reader, nothing in process memory ordering the two — the
        // CLI-vs-daemon shape of
        // `appends_from_a_second_store_survive_racing_rewrites`.
        let writer_store = Arc::new(SessionManager::new(temp.path().to_path_buf()));

        let mut doomed = Vec::with_capacity(DOOMED);
        for i in 0..DOOMED {
            let s = deleter_store
                .create_session(
                    PathBuf::from("/tmp/delete_race"),
                    format!("doomed-{i}"),
                    SessionType::User,
                )
                .await
                .unwrap();
            deleter_store
                .add_message(&s.id, &umsg(1, &format!("body-{i}")))
                .await
                .unwrap();
            doomed.push(s.id);
        }
        // The session the writer commits into. It is never deleted, so the two
        // tasks contend for the write lock without ever touching the same rows
        // — the failure below can only be lock ordering, never a row conflict.
        let chatty = writer_store
            .create_session(
                PathBuf::from("/tmp/delete_race"),
                "chatty".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let committed = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));

        let writer = {
            let writer_store = Arc::clone(&writer_store);
            let id = chatty.id.clone();
            let stop = Arc::clone(&stop);
            let committed = Arc::clone(&committed);
            let started = Arc::clone(&started);
            tokio::spawn(async move {
                let mut busy = 0usize;
                let mut i = 0i64;
                while !stop.load(Ordering::Relaxed) {
                    let m = umsg(1_000 + i, &format!("chat-{i}"));
                    started.fetch_add(1, Ordering::Relaxed);
                    match writer_store.add_message(&id, &m).await {
                        Ok(_) => {
                            committed.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if e.to_string().contains("database is locked") => busy += 1,
                        Err(e) => panic!(
                            "append failed for a reason other than lock \
                             contention: {e}"
                        ),
                    }
                    i += 1;
                    tokio::task::yield_now().await;
                }
                busy
            })
        };
        let deleter = {
            let deleter_store = Arc::clone(&deleter_store);
            let doomed = doomed.clone();
            let stop = Arc::clone(&stop);
            let committed = Arc::clone(&committed);
            let started = Arc::clone(&started);
            tokio::spawn(async move {
                // Barrier, not a sleep. Left to chance the deleter sometimes
                // ran its whole loop before the writer task was ever scheduled
                // — across twelve free-running runs the overlap swung from 19
                // commits to 686 — so each delete waits for the writer to
                // ENTER its next append before starting.
                //
                // Which edge it waits on is load-bearing, and was measured.
                // Releasing on the writer's *commit* aims every delete at the
                // one moment the writer provably holds no lock and the WAL
                // provably has not moved: detection under the mutation fell to
                // 0-4 failures per 150 and 3 of 16 mutated runs passed
                // outright. Releasing on the writer's *start* aims each delete
                // at a transaction that is about to take the write lock, which
                // is the state the ordering exists to survive.
                let mut seen = started.load(Ordering::Relaxed);
                let before = committed.load(Ordering::Relaxed);
                let mut errors = Vec::new();
                for id in &doomed {
                    // Five seconds is the pool's own `busy_timeout`: a writer
                    // silent for that long is a real fault, not scheduling
                    // noise, and must not be graded as a passing race.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        let now = started.load(Ordering::Relaxed);
                        if now > seen {
                            seen = now;
                            break;
                        }
                        assert!(
                            std::time::Instant::now() < deadline,
                            "the writer stopped appending, so there is no race \
                             left to grade"
                        );
                        tokio::task::yield_now().await;
                    }

                    if let Err(e) = deleter_store.delete_session(id).await {
                        errors.push(format!("{id}: {e}"));
                    }
                    tokio::task::yield_now().await;
                }
                let during = committed.load(Ordering::Relaxed) - before;
                stop.store(true, Ordering::Relaxed);
                (errors, during)
            })
        };
        let (busy_appends, deletes) = tokio::join!(writer, deleter);
        let busy_appends = busy_appends.unwrap();
        let (errors, committed_during) = deletes.unwrap();

        // 1. The write-first lock ordering held. A delete that opens with a
        // WRITE takes the single per-file write lock up front, where the busy
        // handler *does* apply, so losing a race costs it a wait and never an
        // error. A delete that opens with a READ fails right here, instantly.
        assert!(
            errors.is_empty(),
            "{} of {DOOMED} deletes failed (a `database is locked`, code 5 or \
             its SQLITE_BUSY_SNAPSHOT variant 517, here means the transaction \
             read before it wrote): {:?}",
            errors.len(),
            &errors[..errors.len().min(5)]
        );
        // 2. The race really was a race. The barrier already forces one
        // append per delete, so this restates the guarantee end to end: a
        // barrier that is ever weakened, or a writer that dies halfway,
        // cannot quietly leave assertion 1 grading an empty overlap. The bar
        // is half of `DOOMED` rather than all of it because a *busy* append
        // starts without committing, and those are tolerated. Measured range
        // over 26 clean runs: 151 to 2159.
        assert!(
            committed_during * 2 >= DOOMED,
            "only {committed_during} writes committed alongside {DOOMED} \
             deletes ({busy_appends} lost the write lock): too little overlap \
             for this to still be testing anything"
        );
        // 3. Every delete actually landed, and the writer's session did not.
        assert_eq!(
            deleter_store.count_all_sessions().await.unwrap(),
            1,
            "only the writer's session may remain"
        );
        assert!(
            writer_store.get_session(&chatty.id, false).await.is_ok(),
            "the writer's session must be untouched"
        );
    }

    /// #41: the idless soft-interrupt shape — a `Message::user()` minted
    /// mid-turn, persisted, then retained in the in-memory conversation and
    /// yielded. `add_message_adopting_uid` must stamp the store's minted uid
    /// onto the in-memory copy, so a later re-persist of the retained copy is
    /// an idempotent replay instead of a duplicate row under a fresh uid.
    #[tokio::test]
    async fn adopting_uid_keeps_an_idless_soft_interrupt_in_sync_with_storage() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/soft_interrupt"),
                "SoftInterrupt".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        // The soft-interrupt message carries no id when it is persisted.
        let mut m = amsg(chrono::Utc::now().timestamp_millis(), "user correction");
        assert!(m.id.is_none());
        sm.add_message_adopting_uid(&session.id, &mut m)
            .await
            .unwrap();

        let adopted = m.id.clone().expect("the minted uid must be adopted");
        let loaded = sm.get_session(&session.id, true).await.unwrap();
        assert_eq!(
            loaded.conversation.as_ref().unwrap().messages()[0]
                .id
                .as_deref(),
            Some(adopted.as_str()),
            "memory and storage must agree on the uid"
        );

        // Re-persisting the retained copy (e.g. a later conversation write)
        // is now a replay — before adoption it minted a SECOND row.
        sm.add_message_adopting_uid(&session.id, &mut m)
            .await
            .expect("re-persisting the adopted copy must be idempotent");
        assert_eq!(m.id.as_deref(), Some(adopted.as_str()));
        let reloaded = sm.get_session(&session.id, true).await.unwrap();
        assert_eq!(
            reloaded.conversation.unwrap().len(),
            1,
            "the adopted copy must replay, not duplicate"
        );
    }

    /// #41: the replay probe must include `created_timestamp`. Two genuinely
    /// distinct messages that happen to share uid, role, content AND metadata
    /// but were created at different times are NOT replays — collapsing them
    /// would silently drop the second one.
    #[tokio::test]
    async fn same_content_different_created_timestamp_is_not_a_replay() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/created_ts"),
                "CreatedTs".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis();
        let first = sm
            .add_message(&session.id, &amsg(now, "same words").with_id("ts-uid"))
            .await
            .unwrap();
        assert_eq!(first, "ts-uid");

        // Identical role/content/metadata, later creation time: a distinct
        // message, so it must be re-minted and kept — not treated as a replay.
        let second = sm
            .add_message(&session.id, &amsg(now + 5, "same words").with_id("ts-uid"))
            .await
            .expect("a distinct message must be re-minted, not abort");
        assert_ne!(
            second, "ts-uid",
            "a different created_timestamp is a different message, not a replay"
        );

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        assert_eq!(
            loaded.conversation.unwrap().len(),
            2,
            "both distinct messages must be persisted"
        );
    }

    /// #41: `metadata_json` is nullable for rows migrated from older schemas.
    /// The replay probe must decode it as an `Option` and treat NULL as "no
    /// metadata recorded" (matching a default-metadata message) — decoding a
    /// bare String made the probe ERROR on such rows, aborting the very turn
    /// the idempotent-replay path exists to save.
    #[tokio::test]
    async fn replay_against_a_null_metadata_row_still_matches() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/null_md"),
                "NullMd".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis();
        let message = amsg(now, "migrated row").with_id("null-md-uid");
        sm.add_message(&session.id, &message).await.unwrap();

        // Simulate a row migrated from a pre-metadata schema.
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "UPDATE messages SET metadata_json = NULL WHERE session_id = ? AND msg_uid = ?",
        )
        .bind(&session.id)
        .bind("null-md-uid")
        .execute(pool)
        .await
        .unwrap();

        let replay = sm
            .add_message(&session.id, &message)
            .await
            .expect("a NULL-metadata row must not error the replay probe");
        assert_eq!(
            replay, "null-md-uid",
            "the replay must match the migrated row, not re-mint"
        );

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        assert_eq!(
            loaded.conversation.unwrap().len(),
            1,
            "the replay must not duplicate the migrated row"
        );
    }

    #[tokio::test]
    async fn conversation_rewrite_preserves_message_order_when_timestamps_tie() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/message_order"),
                "Message order".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let conversation = Conversation::new_unvalidated(vec![
            umsg(10, "question").with_id("z-user-message"),
            amsg(20, "answer").with_id("a-assistant-message"),
        ]);
        sm.replace_conversation(&session.id, &conversation)
            .await
            .unwrap();

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        let messages = loaded.conversation.unwrap();
        assert_eq!(messages.messages()[0].role, Role::User);
        assert_eq!(messages.messages()[1].role, Role::Assistant);
        let texts = messages
            .messages()
            .iter()
            .map(Message::as_concat_text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["question", "answer"]);
    }

    /// A row an in-flight upgrade left with a NULL `msg_uid` still loads, using
    /// the legacy positional id as a fallback (BR-45 dual-read).
    #[tokio::test]
    async fn get_conversation_falls_back_to_positional_id_for_null_uid() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = sm
            .create_session(
                PathBuf::from("/tmp/br45null"),
                "Original".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        // Insert directly with a NULL msg_uid (mimics a not-yet-backfilled row).
        let pool = sm.storage.pool().await.unwrap();
        sqlx::query(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json, msg_uid) VALUES (?, 'user', ?, 0, '{}', NULL)",
        )
        .bind(&session.id)
        .bind(serde_json::to_string(&vec![MessageContent::text("legacy")]).unwrap())
        .execute(pool)
        .await
        .unwrap();

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        let msg = &loaded.conversation.as_ref().unwrap().messages()[0];
        assert_eq!(
            msg.id.as_deref(),
            Some(format!("msg_{}_0", session.id).as_str())
        );
    }

    /// Two messages sharing a whole-second `created` used to collapse to one
    /// anchor, so a diverge at the first silently carried the second over. The
    /// durable-id anchor keeps only the strict prefix and records the divergence point
    /// (BR-45, item 3).
    #[tokio::test]
    async fn diverge_by_uid_beats_same_second_collision() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = sm
            .create_session(
                PathBuf::from("/tmp/br45"),
                "Original".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        // a1, q2, a2 all share created = 2000 (a single whole second).
        for m in [
            umsg(1000, "q1"),
            amsg(2000, "a1"),
            umsg(2000, "q2"),
            amsg(2000, "a2"),
        ] {
            sm.add_message(&session.id, &m).await.unwrap();
        }

        let loaded = sm.get_session(&session.id, true).await.unwrap();
        let a1_uid = loaded
            .conversation
            .as_ref()
            .unwrap()
            .messages()
            .iter()
            .find(|m| m.as_concat_text() == "a1")
            .and_then(|m| m.id.clone())
            .unwrap();

        // Anchored by durable id: keep exactly [q1, a1].
        let by_uid = sm
            .diverge_session_at(&session.id, None, None, Some(a1_uid.clone()))
            .await
            .unwrap();
        let uid_texts: Vec<String> = by_uid
            .conversation
            .as_ref()
            .unwrap()
            .messages()
            .iter()
            .map(|m| m.as_concat_text())
            .collect();
        assert_eq!(uid_texts, vec!["q1".to_string(), "a1".to_string()]);
        // The divergence point is recorded on the child branch.
        assert_eq!(
            by_uid.branch_point_msg_uid.as_deref(),
            Some(a1_uid.as_str())
        );

        // The legacy timestamp anchor (2000) cannot disambiguate and carries a2
        // over — the very over-truncation the uid anchor fixes.
        let by_ts = sm
            .diverge_session(&session.id, None, Some(2000))
            .await
            .unwrap();
        assert_eq!(by_ts.message_count, 4);
    }

    /// Migration 14 backfills `msg_uid` deterministically from the durable
    /// rowid (`m` || id) and adds the branch divergence-point column.
    #[tokio::test]
    async fn migration_14_backfills_msg_uid_from_rowid() {
        let temp_dir = TempDir::new().unwrap();
        let db = temp_dir.path().join("v13.db");
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .unwrap();

        // A minimal pre-migration (v13) shape: no msg_uid, no branch column.
        sqlx::query("CREATE TABLE sessions (id TEXT PRIMARY KEY, diverged_from TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_timestamp INTEGER NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                metadata_json TEXT
            )"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp) VALUES ('s', 'user', '[]', 0), ('s', 'assistant', '[]', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        SessionStorage::apply_migration(&pool, 14).await.unwrap();

        let uids: Vec<(i64, Option<String>)> =
            sqlx::query_as("SELECT id, msg_uid FROM messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(uids[0].1.as_deref(), Some("m1"));
        assert_eq!(uids[1].1.as_deref(), Some("m2"));

        // The branch divergence-point column now exists and defaults to NULL.
        let bp: Vec<(Option<String>,)> =
            sqlx::query_as("SELECT branch_point_msg_uid FROM sessions")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(bp.is_empty());
    }

    // ---- BR-17: FTS5 relevance-ranked chat recall ----

    #[tokio::test]
    async fn chat_recall_fts_ranks_by_relevance_not_recency() {
        // The exact case the old recency `LIKE` scan got wrong: an older
        // session that matches every query term must outrank a newer session
        // that matches only one of them.
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let relevant = sm
            .create_session(
                PathBuf::from("/tmp/a"),
                "relevant".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(
            &relevant.id,
            &umsg(
                1,
                "quantum entanglement experiment results were significant",
            ),
        )
        .await
        .unwrap();

        // Added later, so it is the more *recent* session.
        let recent = sm
            .create_session(PathBuf::from("/tmp/b"), "recent".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(
            &recent.id,
            &umsg(2, "quantum mechanics is a broad topic in physics"),
        )
        .await
        .unwrap();

        let res = sm
            .search_chat_history(
                "quantum entanglement experiment",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap();

        assert_eq!(res.results.len(), 2, "both sessions mention 'quantum'");
        assert_eq!(
            res.results[0].session_id, relevant.id,
            "the fully-matching session must rank first under bm25"
        );
    }

    #[tokio::test]
    async fn chat_recall_fts_sanitizes_operator_query() {
        // A query containing FTS operators must not raise a syntax error; it is
        // treated as literal terms.
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let s = sm
            .create_session(PathBuf::from("/tmp/a"), "s".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(&s.id, &umsg(1, "the CFTR gene and cystic fibrosis"))
            .await
            .unwrap();

        // Would be a malformed MATCH expression if passed through unsanitized.
        let res = sm
            .search_chat_history(
                "CFTR AND (fibrosis*",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap();
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].session_id, s.id);
    }

    #[tokio::test]
    async fn chat_recall_fts_stays_in_sync_on_replace_conversation() {
        // The compaction/edit rewrite (DELETE + reinsert) must keep the FTS
        // index consistent — the old text drops out, the new text is findable.
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let s = sm
            .create_session(PathBuf::from("/tmp/a"), "s".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(&s.id, &umsg(1, "photosynthesis in chloroplasts"))
            .await
            .unwrap();

        assert_eq!(
            sm.search_chat_history(
                "photosynthesis",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap()
            .results
            .len(),
            1
        );

        // Rewrite the conversation with entirely different text.
        let convo = Conversation::new_unvalidated(vec![umsg(2, "glycolysis in the cytoplasm")]);
        sm.replace_conversation(&s.id, &convo).await.unwrap();

        // Old term is gone, new term is present — index tracked the rewrite.
        assert!(sm
            .search_chat_history(
                "photosynthesis",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap()
            .results
            .is_empty());
        assert_eq!(
            sm.search_chat_history(
                "glycolysis",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap()
            .results
            .len(),
            1
        );
    }

    #[tokio::test]
    async fn migration_15_backfills_fts_index() {
        // Production upgrade: a pre-v15 DB with existing messages gets an FTS
        // index built by migration 15's backfill, so recall works on history
        // that predates the feature.
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        let content_json =
            serde_json::to_string(&vec![MessageContent::text("mitochondria powerhouse cell")])
                .unwrap();

        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();

            sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)")
                .execute(&pool).await.unwrap();
            for v in 1..=7 {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(v)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query(
                r#"CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
                    user_set_name BOOLEAN DEFAULT FALSE, session_type TEXT NOT NULL DEFAULT 'user', working_dir TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    extension_data TEXT DEFAULT '{}', total_tokens INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                    accumulated_total_tokens INTEGER, accumulated_input_tokens INTEGER, accumulated_output_tokens INTEGER,
                    schedule_id TEXT, workflow_json TEXT, user_workflow_values_json TEXT, provider_name TEXT, model_config_json TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query(
                r#"CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
                    role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL,
                    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, tokens INTEGER, metadata_json TEXT
                )"#,
            ).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO sessions (id, name, working_dir) VALUES ('20240101_1', 'old', '/tmp/old')")
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO messages (session_id, role, content_json, created_timestamp) VALUES ('20240101_1', 'user', ?, 1)")
                .bind(&content_json)
                .execute(&pool).await.unwrap();
            pool.close().await;
        }

        // Opening the real manager migrates 8→16, including the FTS backfill.
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let res = sm
            .search_chat_history(
                "mitochondria",
                None,
                None,
                None,
                None,
                crate::session::chat_history_search::SearchReach::tier_only(
                    crate::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap();
        assert_eq!(res.results.len(), 1, "backfilled message is searchable");
        assert_eq!(res.results[0].session_id, "20240101_1");
    }

    #[tokio::test]
    async fn chat_recall_falls_back_to_like_without_fts_table() {
        // A DB lacking messages_fts (older/partial migration) must still return
        // recall results via the legacy `LIKE` scan rather than erroring.
        use crate::session::chat_history_search::ChatHistorySearch;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("nofts.db");
        let content_json =
            serde_json::to_string(&vec![MessageContent::text("ribosome translation")]).unwrap();

        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        // `name` is present because migration arm 4 adds it, and every pool the
        // search can ever see has been through `run_migrations` (or
        // `create_schema`) via `SessionStorage::pool`. `messages_fts` only
        // arrives in arm 15, so a database can lack the FTS index and still have
        // `name` — that gap is exactly what this test exercises. What it must
        // NOT do is model a pre-arm-4 shape: this fixture used to omit `name`
        // entirely, which is unreachable in production and quietly pinned the
        // recall query to the dead `description` column.
        sqlx::query(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, name TEXT DEFAULT '', description TEXT DEFAULT '', working_dir TEXT DEFAULT '', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, role TEXT, content_json TEXT, created_timestamp INTEGER, timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO sessions (id, name) VALUES ('s1', 'Ribosome notes')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages (session_id, role, content_json, created_timestamp) VALUES ('s1', 'user', ?, 1)")
            .bind(&content_json)
            .execute(&pool)
            .await
            .unwrap();

        // Full reach: this fixture's hand-built schema predates `privacy_tier`
        // entirely, which is the whole point — an un-migrated DB is what drives
        // the `LIKE` fallback.
        let res = ChatHistorySearch::new(
            &pool,
            "ribosome",
            None,
            None,
            None,
            None,
            crate::session::chat_history_search::SearchReach::tier_only(
                crate::privacy::ProviderTier::Private,
            ),
        )
        .execute()
        .await
        .unwrap();
        assert_eq!(
            res.results.len(),
            1,
            "LIKE fallback still finds the message"
        );
        assert_eq!(res.results[0].session_id, "s1");
        assert_eq!(
            res.results[0].session_description, "Ribosome notes",
            "the LIKE fallback must name the session too, not just the FTS path"
        );
    }

    #[tokio::test]
    async fn parent_session_id_round_trips() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());

        let parent = manager
            .create_session(
                temp.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let child = manager
            .create_session(
                temp.path().to_path_buf(),
                "child".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();

        // Normally-created sessions carry no parent.
        assert_eq!(child.parent_session_id, None);

        manager
            .update(&child.id)
            .parent_session_id(Some(parent.id.clone()))
            .apply()
            .await
            .unwrap();

        let mut child_message = Message::user().with_text("make the child listable");
        manager
            .add_message_adopting_uid(&child.id, &mut child_message)
            .await
            .unwrap();

        let listed = manager
            .list_sessions_by_types(&[SessionType::SubAgent])
            .await
            .unwrap();
        let listed_child = listed
            .iter()
            .find(|session| session.id == child.id)
            .expect("child is returned by the full-session listing");
        assert_eq!(
            listed_child.parent_session_id.as_deref(),
            Some(parent.id.as_str()),
            "the tolerant row reader must not hide a missing SELECT column"
        );

        let reread = manager.get_session(&child.id, false).await.unwrap();
        assert_eq!(reread.parent_session_id, Some(parent.id));
    }

    // ---- Issue #56: the classification ratchet and its schema ---------------

    /// A bare pool on an existing database file, bypassing `SessionStorage` so a
    /// test can inspect (or damage) the schema the production path produced
    /// without tripping the lazy migration in `SessionStorage::pool`.
    async fn raw_pool(db: &Path) -> Pool<Sqlite> {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(db)
                    .create_if_missing(false),
            )
            .await
            .unwrap()
    }

    async fn column_exists(db: &Path, table: &str, column: &str) -> bool {
        let pool = raw_pool(db).await;
        let found = SessionStorage::table_has_column(&pool, table, column)
            .await
            .unwrap();
        pool.close().await;
        found
    }

    async fn table_exists(db: &Path, table: &str) -> bool {
        let pool = raw_pool(db).await;
        let found = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        pool.close().await;
        found
    }

    /// Stamp the version counter, replacing whatever the ladder recorded.
    /// `get_schema_version` reads `MAX(version)`, so the old rows have to go.
    async fn force_schema_version(db: &Path, version: i32) {
        let pool = raw_pool(db).await;
        sqlx::query("DELETE FROM schema_version")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (?1)")
            .bind(version)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    /// A database whose `sessions` table is exactly the production shape MINUS
    /// issue #56's additions — what a build that predates this task leaves on
    /// disk. Derived from the real DDL and then cut back, rather than
    /// hand-rolled: nine hand-rolled `CREATE TABLE sessions` fixtures already
    /// live in this file and several are two columns wide, so a fixture DDL
    /// cannot witness anything about the real schema. Returns the db path.
    async fn build_pre_privacy_database(data_dir: &Path) -> PathBuf {
        let storage = SessionStorage::create(data_dir).await.unwrap();
        storage.close().await;

        let db = data_dir.join(SESSIONS_FOLDER).join(DB_NAME);
        let pool = raw_pool(&db).await;
        for column in ["privacy_tier", "privacy_reason", "parent_session_id"] {
            if SessionStorage::table_has_column(&pool, "sessions", column)
                .await
                .unwrap()
            {
                sqlx::query(&format!("ALTER TABLE sessions DROP COLUMN {column}"))
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        sqlx::query("DROP TABLE IF EXISTS classification_audit")
            .execute(&pool)
            .await
            .unwrap();
        // A session that predates the migration, so the backfill the ADD COLUMN
        // default performs is observable rather than inferred.
        sqlx::query(
            "INSERT INTO sessions (id, name, working_dir, session_type) VALUES ('old', 'an old chat', '/tmp', 'user')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        db
    }

    #[tokio::test]
    async fn a_fresh_database_defaults_every_session_public() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let s = manager
            .create_session(temp.path().to_path_buf(), "s".into(), SessionType::User)
            .await
            .unwrap();
        assert_eq!(s.privacy_tier, SessionClassification::Public);
        assert_eq!(s.privacy_reason, None);
        assert_eq!(s.parent_session_id, None);

        // The audit table belongs to the fresh path too, and it has to be there
        // in THIS process. `create_schema` runs once, on a database that has no
        // `schema_version` table; the reconcile that also creates the table only
        // runs on a database that already has one, i.e. from the *second* launch
        // onwards. A first-run declassification would otherwise fail with
        // `no such table: classification_audit`. Asserted without reopening
        // through `SessionManager`, since reopening is exactly what would hide
        // the bug.
        let db = temp.path().join(SESSIONS_FOLDER).join(DB_NAME);
        assert!(
            table_exists(&db, "classification_audit").await,
            "a freshly created database is missing classification_audit"
        );
    }

    #[tokio::test]
    async fn the_ratchet_raises_and_no_caller_can_lower_it() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let s = manager
            .create_session(temp.path().to_path_buf(), "s".into(), SessionType::User)
            .await
            .unwrap();

        manager
            .update(&s.id)
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        assert_eq!(
            manager
                .get_session(&s.id, false)
                .await
                .unwrap()
                .privacy_tier,
            SessionClassification::Private
        );

        // The whole audit surface for "can the ratchet be reversed" is this
        // assertion plus one SQL fragment. The storage layer refuses, not the
        // caller — whatever it passes.
        manager
            .update(&s.id)
            .raise_privacy(SessionClassification::Public, "oops")
            .apply()
            .await
            .unwrap();
        let reread = manager.get_session(&s.id, false).await.unwrap();
        assert_eq!(
            reread.privacy_tier,
            SessionClassification::Private,
            "a Public write must be a no-op on a private row"
        );
        // The reason must not be rewritten by the refused write either, or the
        // provenance the declassify dialog grades on (§12.4) is destroyed.
        assert_eq!(reread.privacy_reason.as_deref(), Some("turn:versa_azure"));
    }

    #[tokio::test]
    async fn a_stored_tier_the_reader_refuses_cannot_be_assigned_away() {
        // `SessionClassification::from_stored` maps NULL, `PUBLIC`, and anything
        // else it does not recognise to Private, deliberately and loudly. The SQL
        // has to agree with it on the same predicate, or a row the entire Rust
        // tree treats as private is still assignable to a canonical `public` by
        // any caller — and that write is exactly the reversal the ratchet exists
        // to make impossible. The rule is therefore "only an exactly-`public` row
        // is assignable", not "a `private` row is frozen".
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let s = manager
            .create_session(temp.path().to_path_buf(), "s".into(), SessionType::User)
            .await
            .unwrap();

        // Damage the column the way a hand-edited database, a restored backup or
        // a future writer that forgot `as_sql()` would.
        let db = temp.path().join(SESSIONS_FOLDER).join(DB_NAME);
        let pool = raw_pool(&db).await;
        sqlx::query(
            "UPDATE sessions SET privacy_tier = 'PUBLIC', privacy_reason = 'turn:versa_azure' WHERE id = ?1",
        )
        .bind(&s.id)
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        assert_eq!(
            manager
                .get_session(&s.id, false)
                .await
                .unwrap()
                .privacy_tier,
            SessionClassification::Private,
            "the reader fails closed on a non-canonical value"
        );

        manager
            .update(&s.id)
            .raise_privacy(SessionClassification::Public, "oops")
            .apply()
            .await
            .unwrap();

        manager.close().await;
        let pool = raw_pool(&db).await;
        let (tier, reason): (String, Option<String>) =
            sqlx::query_as("SELECT privacy_tier, privacy_reason FROM sessions WHERE id = ?1")
                .bind(&s.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        pool.close().await;
        assert_eq!(
            tier, "PUBLIC",
            "a value the reader refuses must not become one it accepts"
        );
        assert_eq!(reason.as_deref(), Some("turn:versa_azure"));
    }

    /// A session already raised by a turn, then reached into a private data
    /// source. Both orderings live here because the pair is the whole property.
    async fn reason_after(first: &str, second: &str) -> Option<String> {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let s = manager
            .create_session(temp.path().to_path_buf(), "s".into(), SessionType::User)
            .await
            .unwrap();
        for reason in [first, second] {
            manager
                .update(&s.id)
                .raise_privacy(SessionClassification::Private, reason)
                .apply()
                .await
                .unwrap();
        }
        let row = manager.get_session(&s.id, false).await.unwrap();
        assert_eq!(row.privacy_tier, SessionClassification::Private);
        row.privacy_reason
    }

    #[tokio::test]
    async fn a_later_mcp_event_escalates_the_recorded_provenance() {
        // §12.4 grades the declassification confirmation on whether a private
        // data source was ever REACHED (`mcp:*`) or whether the session merely
        // ran a turn against a private endpoint (`turn:*`) — typed confirmation
        // versus single-click-with-undo. Gate B raises at reply entry and Gate C
        // on dispatch, and Gate C can only fire in a session whose bound
        // provider is already private, so `turn:*` ALWAYS lands first and every
        // `mcp:` event arrives at a row that is already private. A reason frozen
        // on the first raise therefore hides every `mcp:` event there has ever
        // been, and an OMOP cohort session is offered the weak control.
        assert_eq!(
            reason_after("turn:versa_azure", "mcp:ucsfomopagent").await,
            Some("mcp:ucsfomopagent".to_string())
        );
    }

    #[tokio::test]
    async fn a_later_turn_does_not_erase_an_mcp_provenance() {
        // The other ordering, and the reason this is dominance rather than
        // last-write-wins: once a private data source has been reached, no
        // number of ordinary turns afterwards may grade the session back down to
        // "text was only sent to a private endpoint".
        assert_eq!(
            reason_after("mcp:ucsfomopagent", "turn:versa_azure").await,
            Some("mcp:ucsfomopagent".to_string())
        );
        // And within the dominant class the first event still stands, so the
        // provenance names the source that was actually reached first.
        assert_eq!(
            reason_after("mcp:ucsfomopagent", "mcp:cdwagent").await,
            Some("mcp:ucsfomopagent".to_string())
        );
    }

    #[tokio::test]
    async fn every_projection_that_builds_a_session_reads_the_column() {
        // The fail-closed reader means a MISSED projection reads Private, so a
        // test that only checks a private row passes a broken projection. Seed a
        // known-PUBLIC row and assert each projection does not default it.
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let s = manager
            .create_session(temp.path().to_path_buf(), "s".into(), SessionType::User)
            .await
            .unwrap();
        // Both listings INNER JOIN messages, so an empty session is invisible.
        manager
            .add_message(&s.id, &Message::user().with_text("hello"))
            .await
            .unwrap();

        // `privacy_reason` needs its own sentinel, and it needs one on a row that
        // is public. Its reader IS tolerant (`try_get(..).ok().flatten()`), so a
        // projection that drops it hands back `None` and every assertion about
        // the tier still passes — the exact silent shape the fail-closed tier
        // read exists to avoid, one column to the right. A public row carrying a
        // reason is not a contrivance either: it is what §12.5 leaves behind
        // after a declassification (`declassified_by_user`), and the reason is
        // the only remaining record of what the session had been. Seeded in SQL
        // rather than through the builder so the projection test does not also
        // depend on the ratchet's semantics.
        let db = temp.path().join(SESSIONS_FOLDER).join(DB_NAME);
        let pool = raw_pool(&db).await;
        sqlx::query("UPDATE sessions SET privacy_reason = 'declassified_by_user' WHERE id = ?1")
            .bind(&s.id)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let fetched = manager.get_session(&s.id, false).await.unwrap();
        assert_eq!(
            fetched.privacy_tier,
            SessionClassification::Public,
            "get_session"
        );
        assert_eq!(
            fetched.privacy_reason.as_deref(),
            Some("declassified_by_user"),
            "get_session dropped privacy_reason"
        );
        let listed = manager
            .list_sessions_by_types(&[SessionType::User])
            .await
            .unwrap();
        let listed = listed.iter().find(|x| x.id == s.id).unwrap();
        assert_eq!(
            listed.privacy_tier,
            SessionClassification::Public,
            "list_sessions_by_types"
        );
        assert_eq!(
            listed.privacy_reason.as_deref(),
            Some("declassified_by_user"),
            "list_sessions_by_types dropped privacy_reason"
        );
        // `SessionSummary` deliberately carries only the tier — the sidebar
        // badges, it does not explain — so there is no reason to assert here.
        let summaries = manager
            .list_session_summaries(50, 0, false, false)
            .await
            .unwrap();
        assert_eq!(
            summaries
                .iter()
                .find(|x| x.id == s.id)
                .unwrap()
                .privacy_tier,
            SessionClassification::Public,
            "list_session_summaries"
        );
    }

    /// BR-45 + the chat-kind glyphs. The sidebar draws a different icon for a
    /// BRANCH, and `SessionSummary` is the only shape it ever sees.
    ///
    /// ⚠ **The read is tolerant on purpose, which is exactly why this test
    /// exists.** `try_get("diverged_from").ok().flatten()` means a SELECT that
    /// stops listing the column yields `None` rather than erroring — so
    /// dropping `s.diverged_from` from the summary query would compile, run,
    /// return every row, and silently redraw every branch as an ordinary chat.
    /// Nothing else in the suite would notice.
    ///
    /// The signal it replaced was the branch's default NAME, and the slack
    /// there is not hypothetical: on the machine this landed from, 5 of 25 real
    /// branches had been renamed and so drew as plain chats. This test renames
    /// its branch for that reason — a name-based implementation passes only if
    /// the name still says "branch".
    #[tokio::test]
    async fn a_summary_carries_the_branch_lineage_the_sidebar_draws_from() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());

        let parent = manager
            .create_session(
                temp.path().to_path_buf(),
                "Parent".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let branch = manager
            .create_session(
                temp.path().to_path_buf(),
                "My Branch".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        manager
            .update(&branch.id)
            .diverged_from(Some(parent.id.clone()))
            .apply()
            .await
            .unwrap();

        // The summary listing INNER JOINs messages, so an empty session is
        // invisible to it and every assertion below would vacuously find
        // nothing to check.
        for id in [&parent.id, &branch.id] {
            manager
                .add_message(id, &Message::user().with_text("hello"))
                .await
                .unwrap();
        }

        let summaries = manager
            .list_session_summaries(50, 0, false, false)
            .await
            .unwrap();

        let branch_row = summaries
            .iter()
            .find(|x| x.id == branch.id)
            .expect("the branch is missing from the summary listing");
        assert_eq!(
            branch_row.diverged_from.as_deref(),
            Some(parent.id.as_str()),
            "the summary dropped the lineage the sidebar needs to draw a branch"
        );

        let parent_row = summaries
            .iter()
            .find(|x| x.id == parent.id)
            .expect("the parent is missing from the summary listing");
        assert_eq!(
            parent_row.diverged_from, None,
            "an ordinary chat must not claim a lineage it does not have"
        );
    }

    #[tokio::test]
    async fn the_reconcile_adds_the_columns_even_when_the_version_says_it_already_ran() {
        // O10. The plan wrote this against `CURRENT_SCHEMA_VERSION = 16` and a
        // BR-71 branch that shipped its own `17 =>`. BR-71 has since LANDED —
        // this tree already carries 17 and `parent_session_id` — so the hazard
        // is the same one stated in the present tense: a database whose version
        // counter already stands at (or past) the privacy arm's number would
        // SKIP a numbered-arm-only implementation of this task entirely.
        let temp = tempfile::TempDir::new().unwrap();
        let db = build_pre_privacy_database(temp.path()).await;
        force_schema_version(&db, CURRENT_SCHEMA_VERSION).await;
        assert!(!column_exists(&db, "sessions", "privacy_tier").await);

        let manager = SessionManager::new(temp.path().to_path_buf());
        // The constructor is lazy: the schema work happens on the first pool
        // acquisition, so the open has to be forced.
        manager.list_sessions().await.unwrap();

        assert!(column_exists(&db, "sessions", "privacy_tier").await);
        assert!(column_exists(&db, "sessions", "privacy_reason").await);
        assert!(column_exists(&db, "sessions", "parent_session_id").await);
        assert!(table_exists(&db, "classification_audit").await);

        // And the row that was already there is public, not NULL — an existing
        // session is not retroactively private just because the column arrived.
        manager.close().await;
        let pool = raw_pool(&db).await;
        let tier: String = sqlx::query_scalar("SELECT privacy_tier FROM sessions WHERE id = 'old'")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert_eq!(tier, SessionClassification::Public.as_sql());
    }

    #[tokio::test]
    async fn a_competing_migrator_cannot_slip_between_the_shape_check_and_the_alter() {
        // The desktop daemon, a terminal `biorouter` and a scheduled job all
        // open the same database, and the first launch after an upgrade is
        // exactly when they are most likely to start together. A bare
        // check-then-ALTER lets both observe the same missing column before
        // either adds it, and the loser aborts startup with `duplicate column
        // name` — a migration failure, not a retryable one.
        //
        // Deterministic rather than a race: the competing migrator takes the
        // write lock FIRST and holds it, so the reconcile under test is parked
        // at a known point, and only then are the columns added under it. Where
        // it parks is the whole property. Outside a transaction it parks at its
        // ALTER, having already decided the column is missing, and resumes into
        // a duplicate. Under BEGIN IMMEDIATE it parks before its shape check,
        // and re-reads a `sessions` table that now has the columns.
        let temp = tempfile::TempDir::new().unwrap();
        let db = build_pre_privacy_database(temp.path()).await;

        let competitor = raw_pool(&db).await;
        let mut held = competitor.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *held)
            .await
            .unwrap();

        let pool = SessionStorage::create_pool(&db);
        let options = pool
            .connect_options()
            .as_ref()
            .clone()
            .busy_timeout(std::time::Duration::from_secs(30));
        pool.set_connect_options(options);
        let reconciling = pool.clone();
        let reconcile =
            tokio::spawn(async move { SessionStorage::ensure_privacy_schema(&reconciling).await });

        // Long enough for the spawned reconcile to reach whichever statement
        // blocks on the write lock. This deliberately extends only the test
        // pool's busy timeout: a loaded Windows runner can delay this task for
        // more than the production five-second budget before the competitor is
        // released, which tests scheduler load rather than migration ordering.
        // A short sleep here can only produce a false pass, never a false failure.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        for (column, sql_type) in [
            ("privacy_tier", "TEXT NOT NULL DEFAULT 'public'"),
            ("privacy_reason", "TEXT"),
            ("parent_session_id", "TEXT"),
        ] {
            sqlx::query(&format!(
                "ALTER TABLE sessions ADD COLUMN {column} {sql_type}"
            ))
            .execute(&mut *held)
            .await
            .unwrap();
        }
        sqlx::query("COMMIT").execute(&mut *held).await.unwrap();
        drop(held);
        competitor.close().await;

        reconcile
            .await
            .unwrap()
            .expect("the reconcile must survive another process adding the columns under it");

        pool.close().await;
        assert!(column_exists(&db, "sessions", "privacy_tier").await);
        assert!(column_exists(&db, "sessions", "privacy_reason").await);
        assert!(table_exists(&db, "classification_audit").await);
    }

    /// Issue #56 §9.3 B1 — the three session-copy paths.
    ///
    /// Every one of them mints a fresh session and then hand-copies a chosen
    /// subset of the parent's metadata onto it. None of them carried
    /// `provider_name` / `model_config` / `privacy_tier`, so a branch of a
    /// private chat resolved its provider through
    /// `restore_provider_from_session`'s `Config::global()` fallback and ran
    /// private history on the user's default *public* model, with no prompt.
    mod derived_session_carry_over {
        use super::*;

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum CopyPath {
            Copy,
            DivergeForEdit,
            Diverge,
        }

        /// A private session bound to `provider`, carrying one complete
        /// exchange so every path has history to carry over.
        async fn private_session_on(provider: &str) -> (TempDir, SessionManager, Session) {
            let temp = TempDir::new().unwrap();
            let manager = SessionManager::new(temp.path().to_path_buf());
            let parent = manager
                .create_session(
                    temp.path().to_path_buf(),
                    "parent".into(),
                    SessionType::User,
                )
                .await
                .unwrap();
            manager
                .add_message(&parent.id, &Message::user().with_text("hello"))
                .await
                .unwrap();
            manager
                .add_message(&parent.id, &Message::assistant().with_text("hi there"))
                .await
                .unwrap();
            manager
                .update(&parent.id)
                .provider_name(provider)
                .model_config(ModelConfig::new("gpt-4o").unwrap())
                .raise_privacy(SessionClassification::Private, "turn:versa_azure")
                .apply()
                .await
                .unwrap();
            let parent = manager.get_session(&parent.id, true).await.unwrap();
            assert_eq!(parent.privacy_tier, SessionClassification::Private);
            assert_eq!(parent.provider_name.as_deref(), Some(provider));
            (temp, manager, parent)
        }

        async fn run_copy(
            manager: &SessionManager,
            path: CopyPath,
            parent: &Session,
        ) -> Result<Session> {
            match path {
                CopyPath::Copy => manager.copy_session(&parent.id, "child".into()).await,
                // A timestamp beyond every stored message, so the edit path's
                // truncation keeps the whole conversation and this test is
                // about the carry-over and nothing else.
                CopyPath::DivergeForEdit => {
                    manager
                        .diverge_session_for_edit(&parent.id, i64::MAX / 2)
                        .await
                }
                CopyPath::Diverge => manager.diverge_session(&parent.id, None, None).await,
            }
        }

        #[tokio::test]
        async fn every_copy_path_carries_the_tier_and_the_provider() {
            // A test on `copy_session` alone passes an implementation that
            // misses the GUI path entirely — which is exactly how this bug
            // shipped: `routes/session.rs`'s `POST /sessions/{id}/diverge`
            // reaches `diverge_session`, and `diverge_session` does NOT call
            // `copy_session`.
            for path in [CopyPath::Copy, CopyPath::DivergeForEdit, CopyPath::Diverge] {
                let (_temp, manager, parent) = private_session_on("versa_azure").await;
                let child = run_copy(&manager, path, &parent).await.unwrap();
                assert_eq!(
                    child.privacy_tier,
                    SessionClassification::Private,
                    "{path:?}"
                );
                assert_eq!(
                    child.provider_name.as_deref(),
                    Some("versa_azure"),
                    "{path:?}"
                );
                assert!(child.model_config.is_some(), "{path:?}");
                let expected_reason = format!("diverged:{}", parent.id);
                assert_eq!(
                    child.privacy_reason.as_deref(),
                    Some(expected_reason.as_str()),
                    "{path:?}"
                );
            }
        }

        /// Export a real session and hand the JSON back to `import_session`,
        /// with `privacy_tier` set to `tier` (or removed when `None`).
        async fn import_json_with_tier(tier: Option<&str>) -> Session {
            let temp = TempDir::new().unwrap();
            let manager = SessionManager::new(temp.path().to_path_buf());
            let source = manager
                .create_session(temp.path().to_path_buf(), "src".into(), SessionType::User)
                .await
                .unwrap();
            manager
                .add_message(&source.id, &Message::user().with_text("hello"))
                .await
                .unwrap();
            let exported = manager.export_session(&source.id).await.unwrap();

            let mut value: serde_json::Value = serde_json::from_str(&exported).unwrap();
            let object = value.as_object_mut().unwrap();
            assert!(
                object.contains_key("privacy_tier"),
                "export must carry the field this test is about"
            );
            match tier {
                Some(t) => {
                    object.insert("privacy_tier".into(), serde_json::Value::from(t));
                }
                None => {
                    object.remove("privacy_tier");
                }
            }
            manager
                .import_session(&serde_json::to_string(&value).unwrap())
                .await
                .unwrap()
        }

        /// ⚠ Only the THIRD assertion discriminates, and the name overstates the
        /// rest — review found it and it is written down rather than renamed,
        /// because Task 25's gate greps for this name verbatim and a rename
        /// there would silently drop a carrier from the count.
        ///
        /// `import_session` raises to `Private.max(imported)`, and `Private` is
        /// the top of a two-element lattice, so the first two rows would pass
        /// against an implementation that ignored the file's field entirely. The
        /// "only raised BY it" half of the name cannot be exercised until a tier
        /// above Private exists. What the third row rules out is the dangerous
        /// implementation — `raise_privacy(imported, …)`, which would let a
        /// hand-edited export declare itself public and be believed.
        #[tokio::test]
        async fn an_import_with_no_tier_is_private_and_one_with_a_tier_is_only_raised_by_it() {
            // Read the imported field ONLY in the raising direction — never as
            // authority to set public. An imported transcript of unknown
            // provenance is sensitive: unlike migration, there is no local
            // evidence to reason from.
            assert_eq!(
                import_json_with_tier(None).await.privacy_tier,
                SessionClassification::Private
            );
            assert_eq!(
                import_json_with_tier(Some("private")).await.privacy_tier,
                SessionClassification::Private
            );
            assert_eq!(
                import_json_with_tier(Some("public")).await.privacy_tier,
                SessionClassification::Private
            );
        }

        /// The body of the LAST `fn <name>(` in `src`. The three copy paths each
        /// appear twice in this file — a thin `SessionManager` wrapper first,
        /// then the real `SessionStorage` implementation — and it is the second
        /// one this test is about.
        fn fn_body(src: &str, name: &str) -> String {
            let needle = format!("fn {name}(");
            let start = src
                .rfind(&needle)
                .unwrap_or_else(|| panic!("no `fn {name}(` in the file"));
            // `get` rather than `&src[..]` throughout: clippy's `string_slice`
            // refuses raw indexing into a `str`, and every offset here comes
            // from a search on the same string, so `None` is unreachable.
            let signature_onwards = src.get(start..).expect("rfind returns a char boundary");
            let open = signature_onwards
                .find('{')
                .unwrap_or_else(|| panic!("no body for fn {name}"));
            let body_onwards = signature_onwards
                .get(open..)
                .expect("find returns a char boundary");
            let mut depth = 0usize;
            for (offset, ch) in body_onwards.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return body_onwards
                                .get(..offset + ch.len_utf8())
                                .expect("char_indices yields char boundaries")
                                .to_string();
                        }
                    }
                    _ => {}
                }
            }
            panic!("unbalanced braces in fn {name}");
        }

        #[test]
        fn no_copy_path_hand_rolls_its_own_builder_any_more() {
            // The enumeration test, aimed at the three functions that matter
            // rather than at all 104 `create_session` call sites.
            let src = std::fs::read_to_string("src/session/session_manager.rs").unwrap();
            for f in [
                "copy_session",
                "diverge_session_for_edit",
                "diverge_session",
            ] {
                let body = fn_body(&src, f);
                assert!(
                    body.contains("create_derived_session"),
                    "{f} does not use the shared helper"
                );
                assert!(
                    !body.contains(".extension_data("),
                    "{f} still hand-rolls its carry-over"
                );
            }
        }
    }

    /// Issue #56 Task 38 — the ONE-TIME backfill, and the numbers the day-one
    /// notice quotes.
    ///
    /// The hazard the first test exists for: the backfill's `WHERE provider_name
    /// IN (…)` is correct exactly once. `declassify` deliberately leaves
    /// `provider_name` untouched (a public chat may run a private model), so the
    /// same statement re-run on a later startup would silently undo the one
    /// irreversible action the design gives the user. That is why it lives in
    /// the numbered arm and not in `ensure_privacy_schema`, which runs on every
    /// launch.
    mod migration_backfill {
        use super::*;

        /// One seed row for [`pre_privacy_database_with`]. Ids are assigned in
        /// order: `s1`, `s2`, …
        #[derive(Clone, Copy)]
        struct Seed {
            provider: Option<&'static str>,
            messages: usize,
        }

        fn session_on(provider: &'static str) -> Seed {
            Seed {
                provider: Some(provider),
                messages: 0,
            }
        }

        fn session_on_with_messages(provider: &'static str) -> Seed {
            Seed {
                provider: Some(provider),
                messages: 1,
            }
        }

        fn session_with_null_provider() -> Seed {
            Seed {
                provider: None,
                messages: 0,
            }
        }

        fn null_provider_with_messages() -> Seed {
            Seed {
                provider: None,
                messages: 1,
            }
        }

        /// A database one migration BELOW the backfill arm, holding `seeds`.
        ///
        /// Named for what it is rather than for a version number: the plan calls
        /// this `migrated_v16_db_with`, but BR-71 landed `17 =>`
        /// (`parent_session_id`) first and Task 6 took 18 for the columns, so the
        /// backfill arm is 19 and "one below the arm under test" is the only
        /// durable way to say this.
        ///
        /// ⚠ It leaves the counter at **18 with the privacy columns dropped** —
        /// a database that has passed the columns arm without having its columns.
        /// That is not a contrivance; it is exactly the state
        /// `the_reconcile_adds_the_columns_even_when_the_version_says_it_already_ran`
        /// exists for, and it is why arm 19 calls `ensure_privacy_schema` again
        /// rather than assuming 18 ran. If that call is ever removed, every test
        /// in this module fails with `no such column: privacy_tier`.
        async fn pre_privacy_database_with(data_dir: &Path, seeds: &[Seed]) -> PathBuf {
            let storage = SessionStorage::create(data_dir).await.unwrap();
            storage.close().await;

            let db = data_dir.join(SESSIONS_FOLDER).join(DB_NAME);
            let pool = raw_pool(&db).await;
            // `parent_session_id` is deliberately NOT dropped: it belongs to
            // BR-71's arm, which every database down here has already applied.
            for column in ["privacy_tier", "privacy_reason"] {
                sqlx::query(&format!("ALTER TABLE sessions DROP COLUMN {column}"))
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query("DROP TABLE IF EXISTS classification_audit")
                .execute(&pool)
                .await
                .unwrap();

            for (index, seed) in seeds.iter().enumerate() {
                let id = format!("s{}", index + 1);
                sqlx::query(
                    "INSERT INTO sessions (id, name, working_dir, session_type, provider_name) \
                     VALUES (?1, ?2, '/tmp', 'user', ?3)",
                )
                .bind(&id)
                .bind(format!("chat {id}"))
                .bind(seed.provider)
                .execute(&pool)
                .await
                .unwrap();

                for n in 0..seed.messages {
                    sqlx::query(
                        "INSERT INTO messages (session_id, role, content_json, created_timestamp) \
                         VALUES (?1, 'user', '[]', ?2)",
                    )
                    .bind(&id)
                    .bind(n as i64)
                    .execute(&pool)
                    .await
                    .unwrap();
                }
            }
            pool.close().await;

            // ⚠ `PRIVACY_BACKFILL_ARM - 1`, NOT `CURRENT_SCHEMA_VERSION - 1`.
            // The doc above says "one below the backfill arm" and that used to be
            // the same number; arm 20 (the repair) made them different, and a
            // database left at 19 would skip the arm every test here is about
            // while still passing several of them — arm 20 runs the same
            // statements. Pinning the fixture to the arm keeps each test honest
            // about which arm it exercised.
            force_schema_version(&db, PRIVACY_BACKFILL_ARM - 1).await;
            db
        }

        fn data_dir_of(db: &Path) -> PathBuf {
            db.parent().unwrap().parent().unwrap().to_path_buf()
        }

        /// A full application open: constructs the manager and forces the lazy
        /// pool, which is what actually runs the migration ladder.
        async fn open(db: &Path) {
            let manager = SessionManager::new(data_dir_of(db));
            manager.list_sessions().await.unwrap();
            manager.close().await;
        }

        async fn row(db: &Path, id: &str) -> Session {
            let manager = SessionManager::new(data_dir_of(db));
            let session = manager.get_session(id, false).await.unwrap();
            manager.close().await;
            session
        }

        /// The real §12.6 writer, reached through the test-only door in
        /// `privacy::declassify`.
        ///
        /// It cannot be called directly from here. `declassify` takes a
        /// proof-of-user token, and
        /// `the_proof_of_user_is_constructed_in_exactly_two_places` fails the
        /// build for any file outside `declassify.rs` and the two door files
        /// that so much as *names* that type — this file must therefore not name
        /// it, which is why the call is one indirection away. (That audit is
        /// whole-file and does not skip comments, and it caught this comment's
        /// first draft.) Hand-rolled SQL is the other alternative and is worse:
        /// a sibling audit permits exactly one tier-lowering `UPDATE` in the
        /// entire tree, so a test copy would have to be composed at runtime to
        /// slip past a security check in order to compile at all.
        async fn declassify_via_user(db: &Path, id: &str) {
            let manager = SessionManager::new(data_dir_of(db));
            let outcome = crate::privacy::declassify::declassify_for_test(&manager, id)
                .await
                .unwrap();
            manager.close().await;
            assert_eq!(
                outcome,
                crate::privacy::declassify::DeclassifyOutcome::Declassified
            );
        }

        async fn count_matching(db: &Path, predicate: &str) -> i64 {
            let pool = raw_pool(db).await;
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM sessions WHERE {predicate}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            pool.close().await;
            count
        }

        /// The stored tier, read WITHOUT opening a `SessionManager`.
        ///
        /// ⚠ `row()` opens the manager, and opening the manager runs the
        /// migration ladder. Every "assert the fixture really is mis-classified
        /// before the repair runs" check therefore has to read the database
        /// directly — `row()` would silently apply the very arm under test and
        /// then report the repaired value, turning the pre-condition into an
        /// assertion that fails for the right-looking wrong reason.
        async fn raw_tier(db: &Path, id: &str) -> String {
            let pool = raw_pool(db).await;
            let tier: String =
                sqlx::query_scalar("SELECT privacy_tier FROM sessions WHERE id = ?1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            pool.close().await;
            tier
        }

        async fn private_count(db: &Path) -> i64 {
            count_matching(db, "IFNULL(privacy_tier, '') <> 'public'").await
        }

        async fn public_count(db: &Path) -> i64 {
            count_matching(db, "IFNULL(privacy_tier, '') = 'public'").await
        }

        async fn notice_counts(db: &Path) -> PrivacyNoticeCounts {
            let pool = raw_pool(db).await;
            let counts = SessionStorage::privacy_notice_counts(&pool).await.unwrap();
            pool.close().await;
            counts
        }

        /// The source of one function in this file, brace-matched.
        ///
        /// ⚠ **Why the four log keys are asserted from the source rather than by
        /// capturing the event, which is what the plan asked for.** A capture was
        /// written first: a `tracing_subscriber` layer behind a thread-local
        /// `set_default`, which is the textbook shape. It passed alone, passed
        /// under `--test-threads=1`, and captured **nothing** from the migration
        /// in the full parallel suite — while an `info!` emitted from the test
        /// body two lines earlier was captured, on the same thread, at
        /// `LevelFilter::TRACE`.
        ///
        /// The difference is that `Interest` is cached **per callsite,
        /// process-globally**, and is computed the first time any thread reaches
        /// that callsite. Every sibling test that opens a `SessionManager`
        /// reaches the migration's `info!` with no subscriber installed, which
        /// caches `Interest::never()`; from then on `event!` short-circuits
        /// before it ever consults this thread's subscriber, and
        /// `rebuild_interest_cache()` does not rescue it. A test built on that
        /// reads a real, correctly-emitted log as absent, and its result depends
        /// on which test won a race. Deterministically wrong is not an option
        /// here, and neither is deleting the assertion — so the fact is split in
        /// two and both halves are checked without touching global state:
        /// [`the_backfill_reports_the_four_counts_it_logs`] pins the four
        /// **values**, and the scan below pins that those four **names** are what
        /// the function hands to `info!`.
        fn fn_body(name: &str) -> String {
            let src = std::fs::read_to_string("src/session/session_manager.rs").unwrap();
            let start = src
                .find(&format!("fn {name}("))
                .unwrap_or_else(|| panic!("no `fn {name}(` in the file"));
            let from_signature = src.get(start..).expect("find returns a char boundary");
            let open = from_signature
                .find('{')
                .unwrap_or_else(|| panic!("no body for fn {name}"));
            let from_body = from_signature
                .get(open..)
                .expect("find returns a char boundary");
            let mut depth = 0usize;
            for (offset, ch) in from_body.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return from_body
                                .get(..offset + ch.len_utf8())
                                .expect("char_indices yields char boundaries")
                                .to_string();
                        }
                    }
                    _ => {}
                }
            }
            panic!("unbalanced braces in fn {name}");
        }

        #[tokio::test]
        async fn the_backfill_cannot_un_declassify() {
            let temp = tempfile::TempDir::new().unwrap();
            let db = pre_privacy_database_with(temp.path(), &[session_on("versa_azure")]).await;

            open(&db).await; // the privacy migration runs, backfilling private
            assert_eq!(
                row(&db, "s1").await.privacy_tier,
                SessionClassification::Private
            );

            declassify_via_user(&db, "s1").await;
            assert_eq!(
                row(&db, "s1").await.privacy_tier,
                SessionClassification::Public
            );
            // Untouched, and that is exactly why the hazard exists.
            assert_eq!(
                row(&db, "s1").await.provider_name.as_deref(),
                Some("versa_azure")
            );

            open(&db).await; // second launch

            assert_eq!(
                row(&db, "s1").await.privacy_tier,
                SessionClassification::Public,
                "the backfill re-privatised a declassified session"
            );
            assert_eq!(
                row(&db, "s1").await.privacy_reason.as_deref(),
                Some("declassified_by_user")
            );
        }

        /// The recovery the fresh-database branch falls back on, exercised
        /// directly because the failure that triggers it cannot be provoked from
        /// outside.
        ///
        /// ⚠ It is asserted through `get_schema_version`, not by counting rows.
        /// That function reads `MAX(version)`, and `update_schema_version`
        /// *appends* — so "walk the counter back" has to mean deleting the rows at
        /// or above the arm, and a `DELETE … WHERE version = N` (or an UPDATE)
        /// would leave the maximum untouched and rewind nothing at all, silently.
        #[tokio::test]
        async fn the_retreat_moves_the_counter_back_into_the_repair_arm() {
            let temp = tempfile::TempDir::new().unwrap();
            let storage = SessionStorage::create(temp.path()).await.unwrap();
            let db = temp.path().join(SESSIONS_FOLDER).join(DB_NAME);
            assert_eq!(
                SessionStorage::get_schema_version(&storage.pool)
                    .await
                    .unwrap(),
                CURRENT_SCHEMA_VERSION,
                "a fresh database must start stamped, or the rewind below is a no-op"
            );

            SessionStorage::retreat_to_privacy_repair(&storage.pool).await;

            assert_eq!(
                SessionStorage::get_schema_version(&storage.pool)
                    .await
                    .unwrap(),
                PRIVACY_REPAIR_ARM - 1,
                "the next launch would not re-enter the repair arm"
            );
            storage.close().await;

            // …and a real open from there does re-enter it and stamps back up,
            // rather than failing on an arm that assumes an earlier one ran.
            open(&db).await;
            let pool = raw_pool(&db).await;
            assert_eq!(
                SessionStorage::get_schema_version(&pool).await.unwrap(),
                CURRENT_SCHEMA_VERSION
            );
            pool.close().await;
        }

        /// The six-provider fixture both count assertions read.
        fn one_of_each() -> [Seed; 6] {
            [
                session_on("versa_azure"),
                session_on("versa_bedrock"),
                session_on("llamacpp"),
                session_on("ollama"),
                session_on("anthropic"),
                session_with_null_provider(),
            ]
        }

        #[tokio::test]
        async fn the_backfill_marks_what_the_data_proves_and_nothing_else() {
            let temp = tempfile::TempDir::new().unwrap();
            let db = pre_privacy_database_with(temp.path(), &one_of_each()).await;

            open(&db).await;

            assert_eq!(private_count(&db).await, 4);
            // NULL provider backfills PUBLIC — fail-open, by decision (DR-10).
            assert_eq!(public_count(&db).await, 2);

            // ...and the buckets are what `info!` is handed. See `fn_body` for
            // why this is a scan and not a captured event.
            let body = fn_body("backfill_privacy_from_recorded_provenance");
            assert!(
                body.contains("info!("),
                "the backfill no longer logs anything"
            );
            for key in [
                "backfilled_private",
                "backfilled_private_from_turn_history",
                "backfilled_declassified_skipped",
                "backfilled_public_named",
                "backfilled_unknown_provider",
                "backfilled_empty",
            ] {
                assert!(
                    body.contains(&format!("{key} =")),
                    "{key} not logged by the backfill"
                );
            }
        }

        /// The values behind those names, on the same fixture.
        ///
        /// Driven through `ensure_privacy_schema` + the backfill directly rather
        /// than through `open`, because the counts describe the state at the
        /// moment the migration runs: a second call reports `private = 0`, having
        /// nothing left to do, and `open` now walks two arms that both call it.
        #[tokio::test]
        async fn the_backfill_reports_the_four_counts_it_logs() {
            let temp = tempfile::TempDir::new().unwrap();
            let db = pre_privacy_database_with(temp.path(), &one_of_each()).await;

            let pool = raw_pool(&db).await;
            SessionStorage::ensure_privacy_schema(&pool).await.unwrap();
            let counts = SessionStorage::backfill_privacy_from_recorded_provenance(&pool)
                .await
                .unwrap();
            pool.close().await;

            assert_eq!(counts.private, 4, "the four private-tier providers");
            assert_eq!(
                counts.private_from_turn_history, 0,
                "this fixture writes no turn ledger, so the second source must find nothing"
            );
            assert_eq!(counts.declassified_skipped, 0);
            assert_eq!(counts.public_named, 1, "anthropic");
            assert_eq!(counts.unknown_provider, 1, "the NULL-provider row");
            assert_eq!(counts.empty, 6, "none of the fixture rows has a message");
        }

        /// Append a turn to the per-turn ledger the way `apply_usage_event` does:
        /// same table, same `provider` column (from the same `get_name()` string
        /// `sessions.provider_name` is bound from), and — load-bearing — a durable
        /// `event_key` and a `billed_total_tokens`.
        ///
        /// ⚠ **A fixture row without those two is not a weaker fixture, it is a
        /// different one.** `reconcile_usage_schema_locked` runs on every startup
        /// and NULLs `provider` on exactly the rows that have neither, so a row
        /// written without them survives the migration arms, gets scrubbed by the
        /// reconcile immediately afterwards, and reads as "this session never ran
        /// that provider" for the rest of the test. That is how the trust
        /// predicate was found; `an_untrustworthy_ledger_row_does_not_classify_anything`
        /// keeps the other half of the behaviour pinned.
        async fn record_turn(db: &Path, session_id: &str, provider: &str, ts: i64) {
            record_turn_row(db, session_id, provider, ts, true).await;
        }

        async fn record_turn_row(
            db: &Path,
            session_id: &str,
            provider: &str,
            ts: i64,
            durable: bool,
        ) {
            let pool = raw_pool(db).await;
            let (event_key, billed) = if durable {
                (Some(format!("{session_id}:{ts}")), Some(10_i64))
            } else {
                (None, None)
            };
            sqlx::query(
                "INSERT INTO token_events \
                     (session_id, ts, total_tokens, model_id, provider, event_key, \
                      billed_total_tokens) \
                 VALUES (?1, ?2, 10, 'm', ?3, ?4, ?5)",
            )
            .bind(session_id)
            .bind(ts)
            .bind(provider)
            .bind(event_key)
            .bind(billed)
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }

        /// The other half of the trust predicate, and it is about real data.
        ///
        /// The old v11/v12 development migrations copied each session's **final**
        /// model backward over its whole history, which is finding 9's own defect
        /// one table over: a per-session last-provider value wearing the shape of
        /// per-turn history. `reconcile_usage_schema_locked` NULLs those rows'
        /// `provider` for exactly that reason — but the numbered arms run
        /// **before** the reconcile on the one upgrade that matters, so the
        /// backfill would see them if it did not exclude them itself.
        #[tokio::test]
        async fn an_untrustworthy_ledger_row_does_not_classify_anything() {
            let temp = tempfile::TempDir::new().unwrap();
            let db =
                pre_privacy_database_with(temp.path(), &[session_on_with_messages("anthropic")])
                    .await;
            // No `event_key`, no `billed_total_tokens` — a row the tree already
            // says is not evidence of the call it sits on.
            record_turn_row(&db, "s1", "ollama", 100, false).await;

            open(&db).await;

            assert_eq!(
                row(&db, "s1").await.privacy_tier,
                SessionClassification::Public,
                "a backward-copied provider value was read as observed turn history"
            );

            // …and the same row WITH a durable identity does classify, so the
            // assertion above is about the identity columns and not about a
            // predicate that matches nothing.
            record_turn_row(&db, "s1", "ollama", 200, true).await;
            force_schema_version(&db, PRIVACY_BACKFILL_ARM).await;
            open(&db).await;

            assert_eq!(
                row(&db, "s1").await.privacy_tier,
                SessionClassification::Private
            );
        }

        /// Issue #56 finding 9 — **the innocent path, and it is asserted against a
        /// row that already exists.**
        ///
        /// A user ran a chat on Ollama and then switched to Claude for one
        /// formatting question. `sessions.provider_name` is the LAST binding, so
        /// it reads `anthropic`, and arm 19's backfill left the chat **public**
        /// with a private transcript in it.
        ///
        /// The fixture is deliberately the state that shipped, reconstructed
        /// rather than assumed: arm 19's own statement is run directly, the row is
        /// confirmed **public** through a raw read, the counter is then set to
        /// `PRIVACY_BACKFILL_ARM`, and only THEN is the repair arm allowed to run.
        /// A test that started from a pre-19 database would prove the fix works
        /// going forward and prove nothing about the rows the finding is about —
        /// which are already on disk, on every machine that has opened this
        /// branch.
        #[tokio::test]
        async fn the_repair_arm_classifies_a_chat_that_switched_away_from_a_private_provider() {
            let temp = tempfile::TempDir::new().unwrap();
            let db = pre_privacy_database_with(
                temp.path(),
                &[
                    session_on_with_messages("anthropic"), // s1: ran on Ollama first
                    session_on_with_messages("anthropic"), // s2: only ever Claude
                ],
            )
            .await;
            record_turn(&db, "s1", "ollama", 100).await;
            record_turn(&db, "s1", "anthropic", 200).await;
            record_turn(&db, "s2", "anthropic", 100).await;

            // Arm 19 only: the state on every machine that has opened this branch.
            force_schema_version(&db, PRIVACY_BACKFILL_ARM - 1).await;
            {
                let pool = raw_pool(&db).await;
                SessionStorage::ensure_privacy_schema(&pool).await.unwrap();
                sqlx::query(&SessionStorage::backfill_update_sql())
                    .execute(&pool)
                    .await
                    .unwrap();
                pool.close().await;
            }
            assert_eq!(
                raw_tier(&db, "s1").await,
                SessionClassification::PUBLIC_SQL,
                "the fixture must reproduce the mis-classification, or the repair below \
                 proves nothing"
            );
            force_schema_version(&db, PRIVACY_BACKFILL_ARM).await;

            open(&db).await; // arm 20, the repair

            let s1 = row(&db, "s1").await;
            assert_eq!(
                s1.privacy_tier,
                SessionClassification::Private,
                "a chat with Ollama turns in the ledger is still classified from its last \
                 binding alone"
            );
            assert_eq!(
                s1.privacy_reason.as_deref(),
                Some("turn:ollama"),
                "the ledger is an OBSERVED turn, so the provenance (and with it §12.4's \
                 declassification grade) must say so"
            );
            // …and the grade that provenance buys is the one the live ratchet
            // would have given the same fact. Read through the real predicate, not
            // restated here.
            assert!(
                !crate::privacy::declassify::requires_typed_confirmation(
                    s1.privacy_reason.as_deref()
                ),
                "an observed private turn is §12.4's weak control"
            );
            assert_eq!(
                row(&db, "s2").await.privacy_tier,
                SessionClassification::Public,
                "a chat whose whole ledger is a public provider must not be swept up"
            );
        }

        /// The repair arm re-runs the same statements a second time, and that is
        /// only safe because of the declassification guard.
        ///
        /// ⚠ This is the test that makes arm 20 possible at all. Arm 19's backfill
        /// was a one-shot statement precisely because `declassify` leaves
        /// `provider_name` in place, so `AND privacy_tier = 'public'` admits every
        /// row the user has just rescued. `the_backfill_cannot_un_declassify`
        /// cannot see that: it opens the database twice, and on the second open
        /// the statement does not run, so it passes whether the guard exists or
        /// not. Here the statement is run **directly**, after a real
        /// declassification, so deleting `NOT_DECLASSIFIED_BY_USER` fails it.
        #[tokio::test]
        async fn the_repair_arm_reruns_the_backfill_without_undoing_a_declassification() {
            let temp = tempfile::TempDir::new().unwrap();
            let db = pre_privacy_database_with(
                temp.path(),
                &[
                    session_on_with_messages("ollama"),    // s1: declassified below
                    session_on_with_messages("anthropic"), // s2: ledger says ollama
                ],
            )
            .await;
            record_turn(&db, "s2", "ollama", 100).await;

            open(&db).await;
            assert_eq!(
                row(&db, "s1").await.privacy_tier,
                SessionClassification::Private
            );
            declassify_via_user(&db, "s1").await;
            declassify_via_user(&db, "s2").await;

            // Not a second `open` — the arms have all run. This calls the
            // statements the way arm 20 does, so the guard is the only thing
            // standing between them and the two rescued rows.
            let counts = {
                let pool = raw_pool(&db).await;
                let counts = SessionStorage::backfill_privacy_from_recorded_provenance(&pool)
                    .await
                    .unwrap();
                pool.close().await;
                counts
            };

            for id in ["s1", "s2"] {
                let session = row(&db, id).await;
                assert_eq!(
                    session.privacy_tier,
                    SessionClassification::Public,
                    "{id}: the re-run re-privatised a session the user declassified"
                );
                assert_eq!(
                    session.privacy_reason.as_deref(),
                    Some("declassified_by_user"),
                    "{id}: the re-run overwrote the declassification provenance"
                );
            }
            assert_eq!(counts.private, 0);
            assert_eq!(counts.private_from_turn_history, 0);
            assert_eq!(
                counts.declassified_skipped, 2,
                "both rows matched the evidence and were held back by the guard, so a 0 here \
                 would mean the statements simply found nothing, which is a different fact"
            );
        }

        /// Issue #56 finding 10 — **legacy JSONL import landed Public and skipped
        /// the backfill entirely.**
        ///
        /// `create_schema` stamps `CURRENT_SCHEMA_VERSION` *before* `import_legacy`
        /// runs, so no numbered arm ever sees the imported rows, and
        /// `import_legacy_session` binds `session.privacy_tier`, which
        /// deserialises from a file predating the column and takes
        /// `#[serde(default = "SessionClassification::public")]`.
        ///
        /// The innocent path is a lost or reset `sessions.db`: every legacy chat
        /// comes back public. This drives the real `SessionManager` open against a
        /// real `.jsonl` on disk — the production path, not the helper.
        #[tokio::test]
        async fn legacy_jsonl_import_classifies_instead_of_landing_public() {
            let temp = tempfile::TempDir::new().unwrap();
            let session_dir = temp.path().join(SESSIONS_FOLDER);
            std::fs::create_dir_all(&session_dir).unwrap();
            // Two legacy chats: one on Ollama, one on Claude. The metadata line is
            // what a pre-privacy build wrote — no `privacy_tier` key at all.
            for (name, provider) in [("20240101_120000", "ollama"), ("20240102_120000", "openai")] {
                std::fs::write(
                    session_dir.join(format!("{name}.jsonl")),
                    format!(
                        "{{\"id\":\"{name}\",\"description\":\"legacy\",\"working_dir\":\"/tmp\",\
                          \"provider_name\":\"{provider}\",\"message_count\":1}}\n\
                         {{\"id\":\"m1\",\"role\":\"user\",\"created\":1704110400,\
                          \"content\":[{{\"type\":\"text\",\"text\":\"hello\"}}]}}\n"
                    ),
                )
                .unwrap();
            }

            let db = session_dir.join(DB_NAME);
            assert!(!db.exists(), "the import path requires a missing database");
            open(&db).await;

            let imported = row(&db, "20240101_120000").await;
            assert_eq!(
                imported.provider_name.as_deref(),
                Some("ollama"),
                "the import did not carry the provider through, so the assertion below would \
                 then pass or fail for an unrelated reason"
            );
            assert_eq!(
                imported.privacy_tier,
                SessionClassification::Private,
                "a legacy Ollama chat came back Public after a database reset"
            );
            assert_eq!(imported.privacy_reason.as_deref(), Some("backfill:ollama"));
            assert_eq!(
                row(&db, "20240102_120000").await.privacy_tier,
                SessionClassification::Public,
                "fail-open (DR-10) still holds for a legacy chat on a public provider"
            );
        }

        /// The other half of finding 10: a database a **buggy build** already
        /// imported into. Those rows are on disk, public, on a database stamped
        /// with the current version — nothing re-enters, so "correct going
        /// forward" leaves exactly them.
        ///
        /// ⚠ The buggy build is **reproduced, not simulated by rewriting the row
        /// afterwards.** `create_schema` + `import_legacy` with no backfill
        /// between them is literally the old code path, called here as its two
        /// halves. The obvious alternative — import through the fixed path, then
        /// `UPDATE … SET privacy_tier = <public>` to undo it — would put a
        /// runtime-composed tier-lowering statement in this file purely to get
        /// past a security audit that permits exactly one, which is a shape this
        /// module already refuses (see `declassify_via_user`).
        #[tokio::test]
        async fn a_database_a_buggy_build_imported_into_is_repaired_by_the_repair_arm() {
            let temp = tempfile::TempDir::new().unwrap();
            let session_dir = temp.path().join(SESSIONS_FOLDER);
            std::fs::create_dir_all(&session_dir).unwrap();
            std::fs::write(
                session_dir.join("20240101_120000.jsonl"),
                "{\"id\":\"20240101_120000\",\"description\":\"legacy\",\
                  \"working_dir\":\"/tmp\",\"provider_name\":\"ollama\",\"message_count\":1}\n\
                 {\"id\":\"m1\",\"role\":\"user\",\"created\":1704110400,\
                  \"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}\n",
            )
            .unwrap();

            let db = session_dir.join(DB_NAME);
            {
                // The old fresh-database branch, exactly: schema (which stamps the
                // counter), then the import, and nothing else.
                let storage = SessionStorage::new(temp.path().to_path_buf());
                SessionStorage::create_schema(&storage.pool).await.unwrap();
                SessionStorage::import_legacy(&storage.pool, &storage.session_dir)
                    .await
                    .unwrap();
                storage.close().await;
            }
            assert_eq!(
                raw_tier(&db, "20240101_120000").await,
                SessionClassification::PUBLIC_SQL,
                "the fixture must reproduce the mis-classification, or the repair below \
                 proves nothing"
            );
            // …and it is stamped, which is the reason nothing re-enters. Forced to
            // the backfill arm because that is where such a database sits once the
            // repair arm exists.
            force_schema_version(&db, PRIVACY_BACKFILL_ARM).await;

            open(&db).await; // arm 20

            assert_eq!(
                row(&db, "20240101_120000").await.privacy_tier,
                SessionClassification::Private,
                "the repair arm left a previously-imported legacy chat public"
            );
        }

        #[tokio::test]
        async fn the_notice_quotes_the_history_visible_count_not_the_raw_one() {
            // `list_sessions_by_types` INNER JOINs messages, so empty sessions
            // never appear. Measured on the operator's machine on 2026-08-03:
            // 654 visible against 1,486 raw would-be-private rows.
            let temp = tempfile::TempDir::new().unwrap();
            let db = pre_privacy_database_with(
                temp.path(),
                &[
                    session_on_with_messages("versa_azure"),
                    session_on_with_messages("versa_azure"),
                    session_on("llamacpp"), // empty: invisible in History
                    session_on_with_messages("anthropic"),
                    session_on_with_messages("anthropic"),
                    session_on_with_messages("anthropic"),
                    null_provider_with_messages(),
                    null_provider_with_messages(),
                ],
            )
            .await;

            open(&db).await;

            let counts = notice_counts(&db).await;
            assert_eq!(
                counts.private_visible, 2,
                "quoted the raw row count instead of the visible one"
            );
            assert_eq!(counts.public_named_visible, 3);
            assert_eq!(counts.unknown_provider_visible, 2);
            assert_eq!(counts.total_visible, 7);
        }

        /// The production slice of this file: everything above the main test
        /// module. This module's own helpers call the backfill directly and would
        /// otherwise count as extra call sites.
        ///
        /// Cut at THIS module's header, not at the first `#[cfg(test)]` in the
        /// file: there are six of those and the first sits near line 627, which
        /// truncates the slice above every symbol these tests are about and turns
        /// a `find` into the failure. (It did.)
        fn production_source() -> String {
            // ⚠ Normalize line endings first. A Windows checkout has CRLF ones
            // (Git for Windows defaults `core.autocrlf` to true and nothing in
            // `.gitattributes` pins `*.rs` to LF), while every needle scanned
            // out of this string — the cut below, and the `\n            }`
            // that closes a match arm in the caller — is written with `\n`. A
            // raw read therefore fails on Windows alone, which is what it did.
            let src = std::fs::read_to_string("src/session/session_manager.rs")
                .unwrap()
                .replace("\r\n", "\n");
            let cut = src
                .find("\n#[cfg(test)]\nmod tests {")
                .expect("this file's main test module moved");
            src.get(..cut)
                .expect("find returns a char boundary")
                .to_string()
        }

        /// Every function reachable from `root` through a `Self::…(` call, as
        /// `(name, body)` pairs including `root` itself.
        ///
        /// ⚠ **Why a transitive walk and not one `fn_body`.** Review found the
        /// hole: `ensure_privacy_schema` is a 25-line `BEGIN IMMEDIATE` wrapper
        /// and every column it adds lives in `ensure_privacy_schema_locked`, so a
        /// scan of the wrapper alone reads none of the DDL. An `UPDATE sessions`
        /// inlined into the `_locked` half would have added no call site and
        /// appeared in no scanned body — it would have passed a guard whose whole
        /// purpose is to police that function. A delegating wrapper is the
        /// obvious shape for the next helper too, so naming one function was
        /// never going to hold; the closure is the property.
        ///
        /// A name with no `fn <name>(` in the production slice is skipped rather
        /// than fatal: `Self::CONST` and associated types appear in the same
        /// syntactic position.
        fn reachable_bodies(production: &str, root: &str) -> Vec<(String, String)> {
            fn body_of(production: &str, name: &str) -> Option<String> {
                let start = production.find(&format!("fn {name}("))?;
                let from_signature = production.get(start..)?;
                let open = from_signature.find('{')?;
                let from_body = from_signature.get(open..)?;
                let mut depth = 0usize;
                for (offset, ch) in from_body.char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return from_body.get(..offset + ch.len_utf8()).map(str::to_string);
                            }
                        }
                        _ => {}
                    }
                }
                None
            }

            let mut seen: std::collections::BTreeSet<String> = Default::default();
            let mut out: Vec<(String, String)> = vec![];
            let mut queue = vec![root.to_string()];
            while let Some(name) = queue.pop() {
                if !seen.insert(name.clone()) {
                    continue;
                }
                let Some(body) = body_of(production, &name) else {
                    continue;
                };
                for (index, _) in body.match_indices("Self::") {
                    let Some(rest) = body.get(index + "Self::".len()..) else {
                        continue;
                    };
                    let callee: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    // `Self::CONST` and associated types share this shape; a
                    // callee is followed by `(`. The identifier is ASCII by
                    // construction (`is_alphanumeric` plus `_` over source
                    // Rust), so `callee.len()` is a char boundary in `rest`.
                    if callee.is_empty()
                        || !rest.get(callee.len()..).is_some_and(|t| t.starts_with('('))
                    {
                        continue;
                    }
                    queue.push(callee);
                }
                out.push((name, body));
            }
            out
        }

        /// The plan's Step 5 gate, as a test rather than a shell command.
        ///
        /// It was written there as two `awk` ranges — "`UPDATE sessions` appears
        /// once in the `17 =>` arm and zero times in `ensure_privacy_schema`" —
        /// and it does not survive contact with this tree three times over. The
        /// arm is **19**: BR-71 landed `17 =>` (`parent_session_id`) first, Task 6
        /// took 18 for the columns, and the backfill needs a number no tester of
        /// this branch has already passed. The statement lives in a named
        /// function the arm calls, so a grep inside the arm's range correctly
        /// reports zero. And scanning `ensure_privacy_schema` alone reads a
        /// delegating wrapper, not the function holding the DDL.
        ///
        /// So the property is asserted directly, and it is the property that
        /// matters: the backfill's production call sites are exactly the two
        /// numbered arms and the fresh-database import, and **nothing reachable
        /// from the per-startup reconcile** performs it or reaches it. Unlike the
        /// shell gate this runs on every CI build, and it cannot pass vacuously —
        /// every range is asserted non-empty first, which is the failure mode the
        /// plan's own gate text warns about at length.
        ///
        /// ⚠ **The third call site is not a relaxation, it is finding 10.**
        /// `create_schema` stamps `CURRENT_SCHEMA_VERSION` before `import_legacy`
        /// runs, so the arms below can never see an imported legacy chat on the
        /// database that imported it. Owning only the arms is precisely the gap
        /// that let a reset `sessions.db` restore a user's whole Ollama history as
        /// public. What must stay true is the *dangerous* half — not per startup —
        /// and that is asserted separately below.
        #[test]
        fn the_backfill_runs_from_the_migration_arms_and_the_import_and_nowhere_else() {
            let production = production_source();

            let arm_body = |version: i32| -> String {
                let header = format!("            {version} => {{");
                let arm_start = production.find(&header).unwrap_or_else(|| {
                    panic!("no `{version} => {{` arm at 12-space indentation, like arms 10..18")
                });
                let after_arm = production
                    .get(arm_start..)
                    .expect("find returns a char boundary");
                let arm_len = after_arm
                    .find("\n            }")
                    .unwrap_or_else(|| panic!("the `{version} =>` arm does not close"));
                let arm = after_arm.get(..arm_len).expect("find yields a boundary");
                assert!(
                    arm.lines().count() > 1,
                    "the `{version} =>` arm range is empty, so every assertion below would \
                     pass vacuously"
                );
                arm.to_string()
            };

            // One definition, three calls. Anything else is a fourth door.
            assert_eq!(
                production
                    .matches("backfill_privacy_from_recorded_provenance(")
                    .count(),
                4,
                "expected exactly one definition and three production call sites of the backfill"
            );
            for version in [PRIVACY_BACKFILL_ARM, PRIVACY_REPAIR_ARM] {
                assert!(
                    arm_body(version).contains("Self::backfill_privacy_from_recorded_provenance("),
                    "the `{version} =>` arm does not run the backfill"
                );
            }
            // ...and the third is the fresh-database branch, identified by the
            // call it sits beside rather than by a line number.
            let pool_body = fn_body("pool");
            assert!(
                pool_body.contains("Self::import_legacy(")
                    && pool_body.contains("Self::backfill_privacy_from_recorded_provenance("),
                "the fresh-database branch imports legacy sessions without classifying them. \
                 a reset `sessions.db` restores every legacy chat as public"
            );

            // ...and NOTHING the per-startup reconcile reaches performs it. This
            // is the whole closure, not one function: see `reachable_bodies`.
            let reconcile = reachable_bodies(&production, "reconcile_loop_schema");
            let names: Vec<&str> = reconcile.iter().map(|(n, _)| n.as_str()).collect();
            for required in [
                "reconcile_loop_schema",
                "ensure_privacy_schema",
                // The half that actually holds the privacy DDL. Named
                // explicitly, because it is the function the previous version of
                // this gate silently failed to read.
                "ensure_privacy_schema_locked",
            ] {
                assert!(
                    names.contains(&required),
                    "`{required}` is not in the per-startup reconcile's reachable set {names:?}: \
                     the walk broke, and every assertion below would pass vacuously"
                );
            }

            for (name, body) in &reconcile {
                assert!(
                    !body.contains("SET privacy_tier"),
                    "`{name}` runs on every startup and assigns `privacy_tier`; a repeated \
                     assignment re-privatises every session the user has declassified, because \
                     declassification deliberately leaves `provider_name` in place"
                );
                assert!(
                    !body.contains("backfill_privacy_from_recorded_provenance"),
                    "`{name}` runs on every startup and reaches the backfill"
                );
            }
        }

        /// The one runtime-composed tier assignment in the tree, pinned.
        ///
        /// ⚠ **Composition is the shape that hides from the audits.**
        /// `exactly_one_statement_in_the_tree_assigns_a_public_classification`
        /// matches one literal spelling line by line and documents that a
        /// statement built from variables is invisible to it. This is the first
        /// `SET privacy_tier` in the tree that is composed at runtime, so it is
        /// the first thing shaped like that bypass. It is benign — it raises, and
        /// it interpolates the constants rather than free text — and this test is
        /// what keeps it that way: the emitted statement is asserted whole, so
        /// flipping the assigned tier, dropping the `AND privacy_tier = 'public'`
        /// ratchet guard, or adding a provider without touching
        /// `BACKFILL_PRIVATE_PROVIDERS` all fail here.
        #[test]
        fn the_backfill_statement_raises_and_nothing_else() {
            let guard = "NOT EXISTS (SELECT 1 FROM classification_audit ca \
                 WHERE ca.session_id = sessions.id AND ca.to_classification = 'public')";
            assert_eq!(
                SessionStorage::backfill_update_sql(),
                format!(
                    "UPDATE sessions \
                        SET privacy_tier = 'private', \
                            privacy_reason = 'backfill:' || provider_name \
                      WHERE provider_name IN ('llamacpp','ollama','versa_azure','versa_bedrock') \
                        AND privacy_tier = 'public' \
                        AND {guard}"
                )
            );
            // The turn-ledger statement, pinned for the same reasons — it is the
            // second runtime-composed tier assignment in the tree, it raises, and
            // it must carry both the ratchet guard and the declassification guard.
            let evidence = "(te.event_key IS NOT NULL OR te.billed_total_tokens IS NOT NULL)";
            assert_eq!(
                SessionStorage::backfill_turn_history_update_sql(),
                format!(
                    "UPDATE sessions \
                        SET privacy_tier = 'private', \
                            privacy_reason = 'turn:' || ( \
                                SELECT te.provider FROM token_events te \
                                 WHERE te.session_id = sessions.id \
                                   AND te.provider IN \
                                   ('llamacpp','ollama','versa_azure','versa_bedrock') \
                                   AND {evidence} \
                                 ORDER BY te.ts ASC, te.id ASC LIMIT 1) \
                      WHERE privacy_tier = 'public' \
                        AND EXISTS (SELECT 1 FROM token_events te \
                                     WHERE te.session_id = sessions.id \
                                       AND te.provider IN \
                                       ('llamacpp','ollama','versa_azure','versa_bedrock') \
                                       AND {evidence}) \
                        AND {guard}"
                )
            );
            // ...and the two ends of the composition are what this file thinks
            // they are, so the literals above cannot drift from the enum.
            assert_eq!(SessionClassification::PRIVATE_SQL, "private");
            assert_eq!(SessionClassification::PUBLIC_SQL, "public");
        }

        /// Neither raising statement may reach a row the user declassified.
        ///
        /// A structural companion to the behavioural test above it: that one
        /// proves the guard works on a real declassified row, this one proves
        /// nobody added a *third* raising statement without it. The guard is
        /// invisible in the outcome when no declassification has happened, which
        /// is every database in CI except the one test that makes one.
        ///
        /// ⚠ **The first block is not decoration.** The obvious form of this test
        /// — `sql.contains(NOT_DECLASSIFIED_BY_USER)` alone — is a tautology
        /// whenever the const itself is the thing that broke: rewriting it to
        /// `"1 = 1"` leaves both statements still "containing the guard" and the
        /// test green. That was written first, and the deliberate-break pass
        /// caught it passing against a backfill that DID re-privatise a
        /// declassified row. So the const's own shape is asserted before it is
        /// used as a needle.
        #[test]
        fn every_raising_statement_carries_the_declassification_guard() {
            let guard = SessionStorage::NOT_DECLASSIFIED_BY_USER;
            for required in [
                "NOT EXISTS",
                "classification_audit",
                "ca.session_id = sessions.id",
                "ca.to_classification = 'public'",
            ] {
                assert!(
                    guard.contains(required),
                    "the guard no longer excludes rows the user declassified; it does not \
                     mention `{required}`: {guard}"
                );
            }

            for sql in [
                SessionStorage::backfill_update_sql(),
                SessionStorage::backfill_turn_history_update_sql(),
            ] {
                assert!(
                    sql.contains("SET privacy_tier"),
                    "the fixture is not a raising statement, so the assertion below is vacuous"
                );
                assert!(
                    sql.contains(guard),
                    "a raising statement without the declassification guard silently reverses \
                     the one irreversible action the design gives the user: {sql}"
                );
            }
        }

        /// The backfill's provider list is a **literal** in SQL, because a
        /// migration must classify rows written by builds other than the one
        /// running it — including `versa_bedrock` rows on a build compiled
        /// without the `aws-providers` feature, which is absent from the live
        /// registry entirely. So it cannot be derived from
        /// `providers::providers()`, and this is what stops it drifting: the set
        /// of provider modules whose shipped metadata claims Private must be
        /// exactly the set the migration privatises.
        #[test]
        fn the_backfilled_provider_set_is_every_provider_that_claims_private() {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/providers");
            let mut declaring: Vec<String> = vec![];
            let mut scanned = 0usize;
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                scanned += 1;
                let src = std::fs::read_to_string(&path).unwrap();
                if src.contains("with_tier(ProviderTier::Private)") {
                    declaring.push(path.file_stem().unwrap().to_string_lossy().to_string());
                }
            }
            assert!(
                scanned >= 30,
                "only {scanned} provider modules were scanned; a broken walk \
                 reports the same empty set as a clean tree"
            );
            declaring.sort();
            let mut backfilled: Vec<String> = SessionStorage::BACKFILL_PRIVATE_PROVIDERS
                .iter()
                .map(|name| (*name).to_string())
                .collect();
            backfilled.sort();
            assert_eq!(
                declaring, backfilled,
                "a provider's shipped tier and the migration's provider list disagree"
            );
        }
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    fn day(n: u32) -> String {
        format!("2026-03-{n:02}")
    }

    /// Linear scaling collapses every ordinary day into the faintest shade as
    /// soon as one marathon day exists. This pins the log+quartile behaviour that
    /// replaced it: ordinary days must occupy more than one level.
    #[test]
    fn one_huge_day_does_not_flatten_the_rest() {
        let mut sessions = Vec::new();
        let mut tokens = Vec::new();
        // 12 ordinary days spanning 20k..150k tokens ...
        for i in 1..=12u32 {
            sessions.push((day(i), 1 + i64::from(i % 3)));
            tokens.push((day(i), 20_000 + i64::from(i) * 11_000, 0, 0, true));
        }
        // ... and one 1.8M-token outlier.
        sessions.push((day(13), 6));
        tokens.push((day(13), 1_800_000, 0, 0, true));

        let w = build_activity_window(day(1), day(13), &sessions, &tokens, &[]);

        assert_eq!(w.days.len(), 13);
        let outlier = w.days.iter().find(|d| d.date == day(13)).unwrap();
        assert_eq!(outlier.level, 4, "the marathon day is the darkest");

        let ordinary: std::collections::BTreeSet<u8> = w
            .days
            .iter()
            .filter(|d| d.date != day(13))
            .map(|d| d.level)
            .collect();
        assert!(
            ordinary.len() >= 3,
            "ordinary days must spread across levels, got {ordinary:?}"
        );
        assert!(!ordinary.contains(&0), "an active day is never level 0");
    }

    #[test]
    fn idle_days_are_omitted_entirely() {
        let sessions = vec![(day(1), 1)];
        let w = build_activity_window(day(1), day(5), &sessions, &[], &[]);
        assert_eq!(w.days.len(), 1);
        assert_eq!(w.days[0].date, day(1));
    }

    /// Messages alone (an edited transcript) must not light a cell — only a
    /// session started or a token spent counts as activity.
    #[test]
    fn messages_alone_do_not_create_an_active_day() {
        let messages = vec![(day(2), 40)];
        let w = build_activity_window(day(1), day(3), &[], &[], &messages);
        assert!(w.days.is_empty());
    }

    #[test]
    fn tokens_lead_sessions_break_ties() {
        // Same session count, more tokens -> strictly higher score.
        assert!(activity_score(1, 200_000) > activity_score(1, 20_000));
        // Same tokens, more sessions -> strictly higher score.
        assert!(activity_score(3, 50_000) > activity_score(1, 50_000));
        // A deep single session outranks three trivial ones.
        assert!(activity_score(1, 500_000) > activity_score(3, 1_000));
        assert_eq!(activity_score(0, 0), 0.0);
    }

    #[test]
    fn streaks_count_consecutive_active_days() {
        // active: 1,2,3   idle: 4   active: 6,7  (5 idle)
        let sessions: Vec<(String, i64)> =
            [1u32, 2, 3, 6, 7].iter().map(|i| (day(*i), 1)).collect();
        let w = build_activity_window(day(1), day(7), &sessions, &[], &[]);
        assert_eq!(w.longest_streak, 3);
        assert_eq!(w.current_streak, 2, "6th and 7th");
    }

    /// A user who has not opened the app *yet today* has not broken their streak.
    #[test]
    fn current_streak_tolerates_an_inactive_today() {
        let sessions: Vec<(String, i64)> = [4u32, 5, 6].iter().map(|i| (day(*i), 1)).collect();
        let w = build_activity_window(day(1), day(7), &sessions, &[], &[]);
        assert_eq!(w.current_streak, 3);
    }

    #[test]
    fn max_sessions_and_tokens_reported() {
        let sessions = vec![(day(1), 2), (day(2), 5)];
        let tokens = vec![(day(1), 900, 400, 500, true), (day(2), 100, 60, 40, true)];
        let w = build_activity_window(day(1), day(2), &sessions, &tokens, &[]);
        assert_eq!(w.max_sessions, 5);
        assert_eq!(w.max_tokens, 900);
        let d1 = &w.days[0];
        assert_eq!((d1.input_tokens, d1.output_tokens), (400, 500));
    }

    #[test]
    fn token_completeness_is_preserved_per_day() {
        let sessions = vec![(day(1), 1), (day(2), 1)];
        let tokens = vec![(day(1), 0, 0, 0, false), (day(2), 500, 400, 100, true)];
        let w = build_activity_window(day(1), day(2), &sessions, &tokens, &[]);

        assert!(!w.tokens_complete);
        assert!(!w.days[0].tokens_complete);
        assert!(w.days[1].tokens_complete);
    }
}

/// #51: the preservation marker must survive every way a session's history is
/// rewritten, copied or moved. A pin that is honoured by compaction but erased
/// by a fork is worse than no pin at all, because callers would trust it.
///
/// `replace_conversation` DELETEs and re-INSERTs every row, and export/import,
/// copy and diverge all funnel through it — so these are one guarantee tested
/// six ways, not six independent guarantees.
#[cfg(test)]
mod pin_persistence_tests {
    use super::*;
    use crate::conversation::message::MessageMetadata;
    use crate::conversation::Conversation;
    use tempfile::TempDir;

    const PIN_TEXT: &str = "NOTE: always cite the 2019 cohort";

    /// A session holding: a plain turn, a PINNED note, another plain turn.
    async fn seeded(sm: &SessionManager) -> String {
        let session = sm
            .create_session(
                PathBuf::from("/tmp/pin"),
                "pin-persistence".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        for (i, message) in [
            Message::user().with_text("first"),
            Message::assistant().with_text("ok"),
            Message::user().with_text(PIN_TEXT).pinned(),
            Message::user().with_text("second"),
            Message::assistant().with_text("done"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut message = message;
            message.created = 1_700_000_000 + i as i64;
            sm.add_message(&session.id, &message).await.unwrap();
        }
        session.id
    }

    async fn pinned_texts(sm: &SessionManager, session_id: &str) -> Vec<String> {
        sm.get_session(session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .filter(|m| m.is_pinned())
            .map(|m| m.as_concat_text())
            .collect()
    }

    /// The baseline: `add_message` → `get_conversation` keeps the marker.
    #[tokio::test]
    async fn a_pin_survives_the_plain_store_round_trip() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        assert_eq!(pinned_texts(&sm, &id).await, vec![PIN_TEXT.to_string()]);
    }

    /// The dangerous one: the whole-history DELETE + re-INSERT that compaction,
    /// message editing and every fork path run.
    #[tokio::test]
    async fn a_pin_survives_a_whole_history_rewrite() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        let current = sm
            .get_session(&id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        sm.replace_conversation(&id, &current).await.unwrap();

        assert_eq!(pinned_texts(&sm, &id).await, vec![PIN_TEXT.to_string()]);
    }

    /// The guarded rewrite from part (a) of #51 — including the FOREIGN-TAIL
    /// path, which decodes recovered rows itself rather than reusing the read
    /// path, and so could drop the marker independently.
    #[tokio::test]
    async fn a_pin_survives_the_guarded_rewrite_and_the_recovered_tail() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
        let known = session.conversation.clone().unwrap();

        // A concurrent writer appends a PINNED note after the snapshot: it is
        // foreign to the rewrite and must come back through `scan_foreign_tail`
        // with its marker intact.
        sm.add_message(
            &id,
            &Message::user()
                .with_text("NOTE: appended mid-rewrite")
                .pinned(),
        )
        .await
        .unwrap();

        // The rewrite drops the tail entirely; only the guard can save the note.
        let shrunk = Conversation::new_unvalidated(known.messages()[..3].to_vec());
        let (outcome, stored) = sm
            .replace_conversation_preserving_tail(&id, &shrunk, basis, &known)
            .await
            .unwrap();
        assert!(
            outcome.stored(),
            "the guarded rewrite must land: {outcome:?}"
        );

        let stored_pins: Vec<String> = stored
            .messages()
            .iter()
            .filter(|m| m.is_pinned())
            .map(|m| m.as_concat_text())
            .collect();
        assert_eq!(
            stored_pins,
            vec![
                PIN_TEXT.to_string(),
                "NOTE: appended mid-rewrite".to_string()
            ],
            "both the kept pin and the recovered foreign pin must stay marked"
        );
        assert_eq!(pinned_texts(&sm, &id).await, stored_pins);
    }

    #[tokio::test]
    async fn a_pin_survives_a_copy() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        let copy = sm.copy_session(&id, "Copy".to_string()).await.unwrap();
        assert_eq!(
            pinned_texts(&sm, &copy.id).await,
            vec![PIN_TEXT.to_string()]
        );
        // And the parent is untouched.
        assert_eq!(pinned_texts(&sm, &id).await, vec![PIN_TEXT.to_string()]);
    }

    #[tokio::test]
    async fn a_pin_survives_a_branch() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        let branch = sm.diverge_session(&id, None, None).await.unwrap();
        assert_eq!(
            pinned_texts(&sm, &branch.id).await,
            vec![PIN_TEXT.to_string()],
            "a branch must inherit the pin, or a note vanishes at the fork"
        );
    }

    /// The edit fork truncates at a timestamp. A pin BEFORE the cut is kept;
    /// this asserts the truncation path does not launder the marker off it.
    #[tokio::test]
    async fn a_pin_survives_a_fork_for_edit() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        // Cut just after the pinned note (created = 1_700_000_002).
        let fork = sm
            .diverge_session_for_edit(&id, 1_700_000_003)
            .await
            .unwrap();
        assert_eq!(
            pinned_texts(&sm, &fork.id).await,
            vec![PIN_TEXT.to_string()]
        );
    }

    #[tokio::test]
    async fn a_pin_survives_export_and_import() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        let exported = sm.export_session(&id).await.unwrap();
        assert!(
            exported.contains("\"pinned\": true"),
            "the marker must be in the exported document, not just in memory"
        );

        let imported = sm.import_session(&exported).await.unwrap();
        assert_eq!(
            pinned_texts(&sm, &imported.id).await,
            vec![PIN_TEXT.to_string()]
        );
    }

    /// An IMPORT of a document written before the marker existed must decode,
    /// with every message unpinned and its visibility intact. This is the
    /// regression the `#[serde(default)]` on `MessageMetadata::pinned` exists
    /// for; without it the whole import fails.
    #[tokio::test]
    async fn a_legacy_export_without_the_marker_still_imports() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let id = seeded(&sm).await;

        let exported = sm.export_session(&id).await.unwrap();
        let legacy = exported.replace(",\n        \"pinned\": true", "");
        let legacy = legacy.replace(",\n        \"pinned\": false", "");
        assert!(!legacy.contains("\"pinned\""), "stripped the marker");

        let imported = sm.import_session(&legacy).await.unwrap();
        let conversation = sm
            .get_session(&imported.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(conversation.messages().len(), 5);
        assert!(conversation.messages().iter().all(|m| !m.is_pinned()));
        assert!(conversation.messages().iter().all(|m| m.is_agent_visible()));
    }

    /// #41's idempotent-replay probe compares the STORED metadata json with the
    /// candidate's. A pin difference is a real difference: re-adding the same
    /// uid with the marker flipped must not be swallowed as a replay.
    #[tokio::test]
    async fn the_replay_probe_treats_a_pin_change_as_a_difference() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                PathBuf::from("/tmp/pin"),
                "replay".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let mut plain = Message::user().with_text("note").with_id("fixed-uid");
        plain.created = 1_700_000_000;
        sm.add_message(&session.id, &plain).await.unwrap();

        // Identical apart from the marker: a distinct message, so it gets its
        // own row under a re-minted uid rather than being dropped as a replay.
        let pinned = plain
            .clone()
            .with_metadata(MessageMetadata::default().with_pinned());
        let uid = sm.add_message(&session.id, &pinned).await.unwrap();
        assert_ne!(uid, "fixed-uid", "a pin change must re-mint, not collapse");

        let conversation = sm
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(conversation.messages().len(), 2);
        assert_eq!(
            conversation
                .messages()
                .iter()
                .filter(|m| m.is_pinned())
                .count(),
            1
        );
    }
}

/// What a deleted chat leaves behind in the side tables keyed by its id, and
/// what the next chat to hold that id inherits from them.
///
/// `delete_session` removed a chat's `messages`, `message_blobs` and — since
/// F10 — `token_events`, and nothing else keyed by its id. Session ids are
/// reissued (`create_session` mints `<day>_<MAX(N)+1>`), so a row that outlives
/// its chat is not merely retained: it is read as the next chat's. These tests
/// cover the recall index (`messages_fts`), the BR-43 checkpoints (rows and
/// shadow repository) and the cross-affiliation grants (DR-26), each through
/// the delete, through `clear_all_sessions`, and through the startup sweep that
/// repairs what earlier builds left behind.
#[cfg(test)]
mod deleted_chat_side_rows_tests {
    use super::*;
    use crate::checkpoint::{CheckpointKind, CheckpointRecord};
    use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
    use crate::privacy::grant;
    use tempfile::TempDir;

    fn stanford() -> Option<ModelAffiliation> {
        Some(ModelAffiliation::institution(InstitutionId::new(
            "stanford",
        )))
    }

    async fn new_chat(sm: &SessionManager, dir: &TempDir) -> String {
        sm.create_session(dir.path().into(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id
    }

    /// Delete `id` and start the next chat, which takes the same id. Asserted,
    /// because a test of what a reissued id inherits proves nothing if the id
    /// did not come back.
    async fn delete_and_reissue(sm: &SessionManager, dir: &TempDir, id: &str) -> String {
        sm.delete_session(id).await.unwrap();
        let next = new_chat(sm, dir).await;
        assert_eq!(
            next, id,
            "the fixture must reproduce the id reuse, or this test proves nothing"
        );
        next
    }

    fn checkpoint(session_id: &str, id: &str) -> CheckpointRecord {
        CheckpointRecord {
            id: id.to_string(),
            session_id: session_id.to_string(),
            turn_index: 0,
            anchor_ts: 1,
            kind: CheckpointKind::PreStep,
            commit_sha: "commit".to_string(),
            tree_sha: "tree".to_string(),
            changed_paths: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    async fn rows(sm: &SessionManager, table: &str, session_id: &str) -> i64 {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE session_id = ?1"
        ))
        .bind(session_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The recall index mirrors what the user saw in the chat, word for word.
    /// `replace_conversation` and `truncate_conversation` already keep it in
    /// step; deleting the chat left all of it on disk — measured on 2026-09-11
    /// against a real daemon, two `messages_fts` rows after an HTTP delete that
    /// had removed every `messages` row. Recall could not reach them (it joins
    /// `messages`, whose rowids are never reissued), but the text of a chat the
    /// user deleted — possibly PHI — stayed in `sessions.db`.
    #[tokio::test]
    async fn deleting_a_chat_deletes_its_recall_index_rows() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let deleted = new_chat(&sm, &dir).await;
        let kept = new_chat(&sm, &dir).await;
        for id in [&deleted, &kept] {
            sm.add_message(id, &Message::user().with_text("patient MRN 12345"))
                .await
                .unwrap();
        }
        assert_eq!(
            rows(&sm, "messages_fts", &deleted).await,
            1,
            "the fixture must index the message, or this test proves nothing"
        );

        sm.delete_session(&deleted).await.unwrap();

        assert_eq!(
            rows(&sm, "messages_fts", &deleted).await,
            0,
            "a deleted chat's text stayed in the recall index"
        );
        assert_eq!(
            rows(&sm, "messages_fts", &kept).await,
            1,
            "the delete is scoped to the chat being deleted"
        );
    }

    /// BR-43. `delete_checkpoints` existed and nothing on the delete path called
    /// it, so the next chat to hold a deleted chat's id listed that chat's
    /// checkpoints as its own — and restoring one would roll the new chat's
    /// working directory and transcript back to a point in a different chat.
    #[tokio::test]
    async fn a_reissued_session_id_does_not_list_the_deleted_chats_checkpoints() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let id = new_chat(&sm, &dir).await;
        sm.insert_checkpoint(&checkpoint(&id, "cp-deleted"))
            .await
            .unwrap();

        let next = delete_and_reissue(&sm, &dir, &id).await;

        assert_eq!(rows(&sm, "checkpoints", &next).await, 0);
        assert!(
            sm.list_checkpoints(&next).await.unwrap().is_empty(),
            "a new chat listed a deleted chat's checkpoints"
        );
    }

    /// The other half of a checkpoint: the shadow git repository at
    /// `<data dir>/checkpoints/<id>/`, which holds snapshots of the chat's working
    /// tree — file contents, not just a record. `CheckpointManager::gc` removes
    /// it "on session delete" and nothing ever called it, so the repository stayed
    /// and the next chat to hold the id opened it and committed on top of it.
    #[tokio::test]
    async fn deleting_a_chat_removes_its_checkpoint_repository() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let deleted = new_chat(&sm, &dir).await;
        let kept = new_chat(&sm, &dir).await;
        let repo = |id: &str| dir.path().join("checkpoints").join(id);
        for id in [&deleted, &kept] {
            std::fs::create_dir_all(repo(id).join("git")).unwrap();
            std::fs::write(repo(id).join("git").join("HEAD"), "ref: refs/heads/main").unwrap();
        }

        sm.delete_session(&deleted).await.unwrap();

        assert!(
            !repo(&deleted).exists(),
            "a deleted chat's checkpoint repository stayed on disk"
        );
        assert!(
            repo(&kept).join("git").join("HEAD").exists(),
            "the delete is scoped to the chat being deleted"
        );
    }

    /// The repository is found by joining the session id onto a directory, so
    /// an id that is not one plain path component must never reach
    /// `remove_dir_all`. No id `create_session` mints can be one, but the column
    /// is free text and a restored or hand-edited database can hold anything:
    /// `.` would name every chat's repository at once and `..` the data
    /// directory itself.
    #[tokio::test]
    async fn an_id_that_is_not_one_path_component_removes_no_directory() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let bystander = dir.path().join("checkpoints").join("20260101_1");
        std::fs::create_dir_all(bystander.join("git")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        let pool = sm.storage().pool().await.unwrap();
        for id in [".", "..", "../outside", "a/b", ""] {
            sqlx::query("INSERT INTO sessions (id, name, working_dir) VALUES (?1, 'odd', '/tmp')")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
            sm.delete_session(id).await.unwrap();
        }

        assert!(
            bystander.join("git").exists(),
            "another chat's repository went"
        );
        assert!(outside.exists(), "a directory outside `checkpoints/` went");
        assert!(dir.path().join(SESSIONS_FOLDER).join(DB_NAME).exists());
    }

    /// Issue #56 DR-26. A reset of History (`clear_all_sessions`) empties every
    /// table keyed by a chat — and left the user's cross-affiliation grants, the
    /// one table in that family that is an authorisation rather than a record.
    /// Ids restart after a wipe, so the first chats created afterwards would
    /// have held acceptances their user never gave.
    #[tokio::test]
    async fn clearing_history_removes_cross_affiliation_grants() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let id = new_chat(&sm, &dir).await;
        grant::record_for_test(&sm, &id, "ucsfomopagent", stanford())
            .await
            .unwrap();

        sm.clear_all_sessions().await.unwrap();

        let pool = sm.storage().pool().await.unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cross_affiliation_grants")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0, "a History reset kept the user's grants");
    }

    /// The rows every earlier build left behind are retired by the reconcile that
    /// runs each time a store is opened — the F10 shape, for the same reasons
    /// `prune_orphaned_token_events` gives, and never by a numbered migration
    /// arm. Pinned here:
    ///
    /// - a grant, recall text and checkpoint whose chat is gone go;
    /// - so do the leftovers a deleted chat left under an id that is live
    ///   again — a grant older than the live chat, and recall text for a
    ///   message the live chat never had — which existence alone cannot see;
    /// - the live chat's own rows are never touched;
    /// - a second open changes nothing.
    #[tokio::test]
    async fn reopening_the_store_retires_side_rows_whose_chat_is_gone() {
        let dir = TempDir::new().unwrap();
        let live = {
            let sm = SessionManager::new(dir.path().to_path_buf());
            let live = new_chat(&sm, &dir).await;
            sm.add_message(
                &live,
                &Message::user().with_text("the live chat's own text"),
            )
            .await
            .unwrap();
            grant::record_for_test(&sm, &live, "ucsfomopagent", stanford())
                .await
                .unwrap();
            sm.insert_checkpoint(&checkpoint(&live, "cp-live"))
                .await
                .unwrap();

            // Grants go through the real writer even here: a second statement
            // that inserts into the grant store would fail
            // `grant::tests::exactly_one_statement_in_the_tree_writes_a_cross_affiliation_grant`,
            // and rightly — that audit cannot tell a fixture from a door.
            //
            // A deleted chat's rows, exactly as the pre-fix delete left them.
            grant::record_for_test(&sm, "gone", "ucsfomopagent", stanford())
                .await
                .unwrap();
            let pool = sm.storage().pool().await.unwrap();
            for statement in [
                "INSERT INTO messages_fts (text, session_id, message_id) \
                 VALUES ('text of a deleted chat', 'gone', 900001)",
                "INSERT INTO checkpoints \
                     (id, session_id, turn_index, anchor_ts, kind, commit_sha, tree_sha) \
                 VALUES ('cp-gone', 'gone', 0, 1, 'pre_step', 'commit', 'tree')",
            ] {
                sqlx::query(statement).execute(pool).await.unwrap();
            }
            // A deleted chat's rows under an id that is live again: a grant
            // recorded a day before the live chat existed...
            grant::record_for_test(&sm, &live, "cdwagent", stanford())
                .await
                .unwrap();
            sqlx::query(
                "UPDATE cross_affiliation_grants SET granted_at = datetime('now', '-1 day') \
                  WHERE session_id = ?1 AND extension = 'cdwagent'",
            )
            .bind(&live)
            .execute(pool)
            .await
            .unwrap();
            // ...and recall text for a message the live chat never had.
            sqlx::query(
                "INSERT INTO messages_fts (text, session_id, message_id) \
                 VALUES ('text of an older chat under this id', ?1, 900002)",
            )
            .bind(&live)
            .execute(pool)
            .await
            .unwrap();
            sm.close().await;
            live
        };

        for open in 1..=2 {
            let sm = SessionManager::new(dir.path().to_path_buf());
            for table in ["cross_affiliation_grants", "messages_fts", "checkpoints"] {
                assert_eq!(
                    rows(&sm, table, "gone").await,
                    0,
                    "open {open}: {table} kept a row whose chat is gone"
                );
            }

            assert!(
                grant::is_granted(&sm, &live, "ucsfomopagent", stanford()).await,
                "open {open}: the live chat's own grant was swept"
            );
            assert_eq!(
                rows(&sm, "cross_affiliation_grants", &live).await,
                1,
                "open {open}: a grant older than the chat holding its id survived"
            );
            let pool = sm.storage().pool().await.unwrap();
            let recall_text: Vec<String> =
                sqlx::query_scalar("SELECT text FROM messages_fts WHERE session_id = ?1")
                    .bind(&live)
                    .fetch_all(pool)
                    .await
                    .unwrap();
            assert_eq!(
                recall_text,
                vec!["the live chat's own text".to_string()],
                "open {open}: the live chat's recall index is exactly its own messages"
            );
            assert_eq!(
                sm.list_checkpoints(&live).await.unwrap().len(),
                1,
                "open {open}: the live chat's checkpoint was swept"
            );
            sm.close().await;
        }
    }

    /// A chat bound to a private provider and still public — the one shape the
    /// issue #56 backfill exists to raise.
    async fn bind_ollama(sm: &SessionManager, id: &str) {
        sm.update(id).provider_name("ollama").apply().await.unwrap();
    }

    async fn tier(sm: &SessionManager, id: &str) -> SessionClassification {
        sm.get_session(id, false).await.unwrap().privacy_tier
    }

    /// Issue #56 §12.5 keeps `classification_audit` when a chat is deleted, by
    /// design: it answers "what has ever been declassified on this machine".
    /// But the backfill's declassification guard (`NOT_DECLASSIFIED_BY_USER`)
    /// asked that question by bare session id, so once the id came back the
    /// deleted chat's declassification shielded the next chat to hold it — a
    /// chat the user never declassified, skipped by a statement whose whole
    /// job is to raise it. The guard now matches the incarnation of the row
    /// the user actually declassified (#51 W3's reuse-proof identity).
    #[tokio::test]
    async fn a_deleted_chats_declassification_does_not_shield_the_next_chat_to_get_its_id() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let id = new_chat(&sm, &dir).await;
        sm.add_message(&id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        sm.update(&id)
            .provider_name("ollama")
            .raise_privacy(SessionClassification::Private, "turn:ollama")
            .apply()
            .await
            .unwrap();
        assert_eq!(
            crate::privacy::declassify::declassify_for_test(&sm, &id)
                .await
                .unwrap(),
            crate::privacy::declassify::DeclassifyOutcome::Declassified
        );

        let next = delete_and_reissue(&sm, &dir, &id).await;
        bind_ollama(&sm, &next).await;

        let pool = sm.storage().pool().await.unwrap();
        let counts = SessionStorage::backfill_privacy_from_recorded_provenance(pool)
            .await
            .unwrap();
        assert_eq!(
            tier(&sm, &next).await,
            SessionClassification::Private,
            "a deleted chat's declassification shielded the next chat to get its id"
        );
        assert_eq!(counts.private, 1);
        assert_eq!(counts.declassified_skipped, 0);
    }

    /// ...while a ledger row written before rows carried an incarnation keeps
    /// the meaning it was written with. It cannot say which incarnation it was
    /// about, so it is not reinterpreted: it still shields the chat whose id it
    /// names, exactly as before. Undoing a declassification on a guess is the
    /// outcome the guard exists to prevent.
    #[tokio::test]
    async fn a_declassification_recorded_without_an_incarnation_still_shields_its_chat() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let id = new_chat(&sm, &dir).await;
        bind_ollama(&sm, &id).await;
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO classification_audit \
                 (session_id, from_classification, to_classification, reason, actor, \
                  actor_kind, app_version) \
             VALUES (?1, 'private', 'public', 'declassified_by_user', 'user', 'user', 'old')",
        )
        .bind(&id)
        .execute(pool)
        .await
        .unwrap();

        let counts = SessionStorage::backfill_privacy_from_recorded_provenance(pool)
            .await
            .unwrap();
        assert_eq!(tier(&sm, &id).await, SessionClassification::Public);
        assert_eq!(counts.declassified_skipped, 1);
    }

    /// Item 5 of the F10 follow-up: a subagent run is its own session, and
    /// deleting the chat that spawned it does not delete it. That is left as it
    /// was — History and `session list --subagents` show an orphan at the top
    /// level "so nothing becomes unreachable", and a run can hold text the user
    /// typed into it — and it is safe from adoption: no later chat is ever
    /// handed the deleted parent's id, so the orphan's `parent_session_id`
    /// cannot come to name an unrelated chat, which would otherwise inherit its
    /// supervision, its approval routing and its place under that chat in
    /// History.
    #[tokio::test]
    async fn a_deleted_chats_subagent_runs_are_kept_and_never_adopted() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let parent = new_chat(&sm, &dir).await;
        let child = sm
            .create_session(dir.path().into(), "run".into(), SessionType::SubAgent)
            .await
            .unwrap()
            .id;
        sm.update(&child)
            .parent_session_id(Some(parent.clone()))
            .apply()
            .await
            .unwrap();
        sm.add_message(&child, &Message::user().with_text("a subagent's task"))
            .await
            .unwrap();

        sm.delete_session(&parent).await.unwrap();
        let next = new_chat(&sm, &dir).await;

        assert_ne!(next, parent, "a later chat was handed the parent's id");
        let orphan = sm.get_session(&child, true).await.unwrap();
        assert_eq!(orphan.parent_session_id.as_deref(), Some(parent.as_str()));
        assert_eq!(orphan.conversation.map(|c| c.len()), Some(1));
    }

    fn suffix(id: &str) -> i64 {
        id.rsplit_once('_')
            .and_then(|(_, n)| n.parse().ok())
            .unwrap_or_else(|| panic!("`{id}` is not `<prefix>_<N>`"))
    }

    fn prefix(id: &str) -> &str {
        id.rsplit_once('_').map(|(p, _)| p).unwrap()
    }

    /// The root cause, closed: an id is never minted twice. Deleting the newest
    /// chat of the day used to hand its id to the next one, and every store
    /// keyed by that id — tables, the daemon's cached agent, the knowledge
    /// base's per-chat selection, the renderer's chat state — read the deleted
    /// chat's state as the new chat's. The per-table deletes above are still
    /// what removes a deleted chat's data; this is what stops any store anyone
    /// adds from inheriting it.
    #[tokio::test]
    async fn deleting_the_newest_chat_does_not_hand_its_id_to_the_next_one() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let first = new_chat(&sm, &dir).await;
        sm.delete_session(&first).await.unwrap();

        let second = new_chat(&sm, &dir).await;

        assert_ne!(second, first, "a deleted chat's id was minted again");
        assert_eq!(prefix(&second), prefix(&first));
        assert_eq!(suffix(&second), suffix(&first) + 1);
    }

    /// ...across a History reset too — the case #51 W3 found, where every id of
    /// the day came back at once — and across reopening the store, since the
    /// daemon, a terminal `biorouter` and a scheduled job each open it afresh.
    #[tokio::test]
    async fn neither_a_history_reset_nor_a_reopen_restarts_the_ids() {
        let dir = TempDir::new().unwrap();
        let last = {
            let sm = SessionManager::new(dir.path().to_path_buf());
            new_chat(&sm, &dir).await;
            let last = new_chat(&sm, &dir).await;
            sm.clear_all_sessions().await.unwrap();
            sm.close().await;
            last
        };

        let sm = SessionManager::new(dir.path().to_path_buf());
        let next = new_chat(&sm, &dir).await;
        assert_eq!(prefix(&next), prefix(&last));
        assert_eq!(
            suffix(&next),
            suffix(&last) + 1,
            "the ids restarted after a reset and a reopen"
        );
    }

    /// A build without the high-water mark still mints `MAX(N)+1` over the rows
    /// it can see, and it shares this file. Whatever it minted, the next id is
    /// above it: the mark is a floor under the existing rule, never a ceiling.
    #[tokio::test]
    async fn the_next_id_is_above_every_id_an_older_build_minted() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let first = new_chat(&sm, &dir).await;
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, name, working_dir) VALUES (?1, 'older build', '/tmp')",
        )
        .bind(format!("{}_7", prefix(&first)))
        .execute(pool)
        .await
        .unwrap();

        let next = new_chat(&sm, &dir).await;
        assert_eq!(suffix(&next), 8);
    }

    /// Ids are minted inside one write transaction each, so concurrent creators
    /// — the desktop daemon, a terminal `biorouter` and a scheduled job share
    /// the file — never receive the same one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_creates_never_mint_the_same_id() {
        let dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let sm = Arc::clone(&sm);
            let working_dir = dir.path().to_path_buf();
            tasks.push(tokio::spawn(async move {
                sm.create_session(working_dir, "c".into(), SessionType::User)
                    .await
                    .unwrap()
                    .id
            }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(task.await.unwrap());
        }
        let distinct: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(distinct.len(), ids.len(), "duplicate ids minted: {ids:?}");
    }
}
