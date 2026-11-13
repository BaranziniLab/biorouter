//! What a chat and the current month have spent, read for the usage limits.
//!
//! A per-chat limit and the monthly limit are checked before every model
//! request, so this read has to agree with the figures the person sees: the
//! cost card in the composer footer (a chat's own ledger rows plus its
//! subagents') and Settings > App > Usage (month to date over
//! [`BILLABLE_USAGE`], deleted chats included, LOCAL calendar month).
//!
//! Tokens are the billed total of every event (input, output, cache read and
//! cache write, which are disjoint), so a token limit counts what the provider
//! billed, not the size of the conversation. An event from before billed totals
//! were recorded counts its input plus output, a lower bound.
//!
//! Dollars are priced through the same grain and pricing path as the usage
//! summary, per prompt band, so cache buckets bill at the cache rate and a
//! request over a long-context threshold at the model's long-context rate.
//! They are `None` when nothing in the span is priced.

use super::{
    resolve_grain_pricing, totals_from_grain_with_pricing, SessionManager, SessionStorage,
    UsageGrainRow, BILLABLE_USAGE,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Spend of one chat (with its subagents) and of the current local month.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpendSnapshot {
    /// Billed tokens of the chat and every chat whose `parent_session_id` is it.
    pub chat_tokens: i64,
    /// Priced dollars for the same rows, `None` when nothing is priced or
    /// dollars were not asked for.
    pub chat_usd: Option<f64>,
    /// `true` when every row behind `chat_usd` was priced, cache included.
    pub chat_usd_complete: bool,
    /// Billed tokens month to date (local calendar month), every billable chat.
    pub month_tokens: i64,
    /// Priced dollars month to date, `None` when nothing is priced or dollars
    /// were not asked for.
    pub month_usd: Option<f64>,
    /// `true` when every row behind `month_usd` was priced, cache included.
    pub month_usd_complete: bool,
    /// The local month the month figures cover, `YYYY-MM`.
    pub month: String,
}

/// The token buckets of one earlier model request, as the ledger holds them.
/// The usage limits estimate the next request from it
/// (`agents::usage_limit::project_request`).
#[derive(Debug, Clone, PartialEq, Eq, Default, sqlx::FromRow)]
pub struct RequestUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub provider: Option<String>,
    pub model_id: Option<String>,
}

impl RequestUsage {
    /// The whole prompt: fresh input plus cache read plus cache write, which
    /// the ledger keeps disjoint.
    pub fn prompt_tokens(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }
}

/// The event key prefix of a compaction's ledger rows: the summarization
/// requests, whose prompt is the whole history being replaced and whose output
/// is the summary. A compaction is not the shape of the chat's next request,
/// so [`SessionManager::last_request_usage`] does not estimate from it. Like
/// any other row, a compaction row is billed spend and counts as a turn.
pub const COMPACTION_EVENT_KEY_PREFIX: &str = "compaction:";

/// A fresh event key for one compaction's ledger rows
/// (`reply_parts::apply_compaction_metrics` gives a second request of the
/// same compaction `<key>:1`, which keeps the prefix).
pub fn compaction_event_key() -> String {
    format!("{COMPACTION_EVENT_KEY_PREFIX}{}", uuid::Uuid::new_v4())
}

/// A ledger row that is a compaction's ([`COMPACTION_EVENT_KEY_PREFIX`]).
const IS_COMPACTION_ROW: &str = "COALESCE(event_key LIKE 'compaction:%', 0)";

/// The columns of [`RequestUsage`] (and `event_key`), for a ledger row that
/// is a model request of the chat's turns or its compactions: side calls
/// (`side:`) and decision calls (`decision:`) are left out, since neither is
/// the shape of the chat's next request.
const REQUEST_USAGE_SQL: &str = "SELECT COALESCE(input_tokens, 0) AS input_tokens,
            COALESCE(output_tokens, 0) AS output_tokens,
            COALESCE(cache_read_tokens, 0) AS cache_read_tokens,
            COALESCE(cache_creation_tokens, 0) AS cache_creation_tokens,
            provider,
            model_id,
            event_key
       FROM token_events
      WHERE (event_key IS NULL
             OR (event_key NOT LIKE 'side:%' AND event_key NOT LIKE 'decision:%'))
        AND COALESCE(input_tokens, 0) + COALESCE(cache_read_tokens, 0)
            + COALESCE(cache_creation_tokens, 0) > 0";

/// The chat's own ledger rows plus its direct subagents'. Both halves of the
/// `OR` are read through `idx_token_events_session`, and the subagents through
/// `idx_sessions_parent`, so a check reads the chat's rows and nothing else.
const CHAT_ROWS_FILTER: &str = "te.session_id = ?1
            OR te.session_id IN (SELECT id FROM sessions WHERE parent_session_id = ?1)";

/// Billed tokens of [`CHAT_ROWS_FILTER`].
fn chat_tokens_sql() -> String {
    format!(
        "SELECT COALESCE(SUM(COALESCE(te.billed_total_tokens,
                                      COALESCE(te.input_tokens, 0) + COALESCE(te.output_tokens, 0))), 0)
           FROM token_events te
          WHERE {CHAT_ROWS_FILTER}"
    )
}

/// The chat's own ledger rows plus its direct subagents', in the shared grain
/// shape (prompt bands included) so they price exactly like the usage summary.
fn chat_grain_sql() -> String {
    super::token_events_grain_sql(CHAT_ROWS_FILTER)
}

/// The current LOCAL calendar month of [`BILLABLE_USAGE`]. The `ts` bound is
/// what an index can serve: SQLite pushes it into both halves of the union, so
/// the ledger half reads this month's rows through `idx_token_events_ts`
/// instead of computing a local date for every row it holds, before every
/// model request. It starts a day before local midnight on the 1st, so no time
/// zone or daylight-saving offset can leave a row of this month out; the
/// `day` test after it decides.
const THIS_MONTH_FILTER: &str =
    "u.ts >= CAST(strftime('%s', strftime('%Y-%m-01', 'now', 'localtime'), 'utc')
                           AS INTEGER) - 86400
            AND substr(u.day, 1, 7) = strftime('%Y-%m', 'now', 'localtime')";

/// Billed tokens of every billable row this month ([`THIS_MONTH_FILTER`]).
fn month_tokens_sql() -> String {
    format!(
        "SELECT COALESCE(SUM(COALESCE(u.billed_total_tokens,
                                      COALESCE(u.input_tokens, 0) + COALESCE(u.output_tokens, 0))), 0)
           FROM {BILLABLE_USAGE}
          WHERE {THIS_MONTH_FILTER}"
    )
}

/// This month's billable rows in the shared priced grain.
fn month_grain_sql() -> String {
    super::billable_usage_grain_sql("''", THIS_MONTH_FILTER)
}

impl SessionManager {
    /// Spend of `session_id` (with its subagents) and of the current month.
    /// `with_usd` prices both; leave it `false` when only token limits are set,
    /// since pricing may consult the network-backed catalog.
    pub async fn spend_snapshot(&self, session_id: &str, with_usd: bool) -> Result<SpendSnapshot> {
        self.storage.spend_snapshot(session_id, with_usd).await
    }

    /// The last model request `session_id` made itself (not a side call, not a
    /// decision, not a subagent's), or `None` before its first.
    ///
    /// Also `None` when a compaction ran after it: the request before a
    /// compaction carried the history the compaction replaced, so the next
    /// request is estimated like a chat's first (the context manager's size of
    /// it, when the caller holds one) until a request after the compaction is
    /// billed. Estimating from either row would hold the chat to the size of a
    /// history it no longer has.
    pub async fn last_request_usage(&self, session_id: &str) -> Result<Option<RequestUsage>> {
        let pool = self.storage.pool().await?;
        Ok(sqlx::query_as::<_, RequestUsage>(&format!(
            "SELECT * FROM ({REQUEST_USAGE_SQL} AND session_id = ?1 ORDER BY id DESC LIMIT 1)
              WHERE NOT {IS_COMPACTION_ROW}"
        ))
        .bind(session_id)
        .fetch_optional(pool)
        .await?)
    }

    /// The last model request any chat made to `provider`'s `model` (not a
    /// compaction), which gives a chat's first request the cache hit rate the
    /// model has been getting.
    pub async fn last_request_usage_for_model(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Option<RequestUsage>> {
        let pool = self.storage.pool().await?;
        Ok(sqlx::query_as::<_, RequestUsage>(&format!(
            "{REQUEST_USAGE_SQL} AND NOT {IS_COMPACTION_ROW}
                AND provider = ?1 COLLATE NOCASE AND model_id = ?2 COLLATE NOCASE
              ORDER BY id DESC LIMIT 1"
        ))
        .bind(provider)
        .bind(model)
        .fetch_optional(pool)
        .await?)
    }
}

impl SessionStorage {
    async fn spend_snapshot(&self, session_id: &str, with_usd: bool) -> Result<SpendSnapshot> {
        let pool = self.pool().await?;

        let chat_tokens: i64 = sqlx::query_scalar(&chat_tokens_sql())
            .bind(session_id)
            .fetch_one(pool)
            .await?;

        let month_tokens: i64 = sqlx::query_scalar(&month_tokens_sql())
            .fetch_one(pool)
            .await?;

        let month: String = sqlx::query_scalar("SELECT strftime('%Y-%m', 'now', 'localtime')")
            .fetch_one(pool)
            .await?;

        let mut snapshot = SpendSnapshot {
            chat_tokens,
            month_tokens,
            month,
            ..SpendSnapshot::default()
        };
        if !with_usd {
            return Ok(snapshot);
        }

        let chat_grain = sqlx::query_as::<_, UsageGrainRow>(&chat_grain_sql())
            .bind(session_id)
            .fetch_all(pool)
            .await?;
        let chat_pricing = resolve_grain_pricing(&chat_grain).await;
        let chat = totals_from_grain_with_pricing(&chat_grain, &chat_pricing);
        snapshot.chat_usd = chat.cost;
        snapshot.chat_usd_complete = !chat.has_unpriced && !chat.cost_excludes_cache;

        let month_grain = sqlx::query_as::<_, UsageGrainRow>(&month_grain_sql())
            .fetch_all(pool)
            .await?;
        let month_pricing = resolve_grain_pricing(&month_grain).await;
        let month_totals = totals_from_grain_with_pricing(&month_grain, &month_pricing);
        snapshot.month_usd = month_totals.cost;
        snapshot.month_usd_complete =
            !month_totals.has_unpriced && !month_totals.cost_excludes_cache;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SessionManager, SessionType, UsageGroup, UsageLedgerEntry};
    use crate::providers::pricing::{prompt_band, LONG_CONTEXT_THRESHOLDS};
    use tempfile::TempDir;

    const SOL: &str = "gpt-5.6-sol-2026-07-09";

    fn sol_turn(
        session_id: &str,
        key: &str,
        input: i32,
        cached: i32,
        output: i32,
    ) -> UsageLedgerEntry {
        let billed = input + cached + output;
        UsageLedgerEntry {
            event_key: key.to_string(),
            session_id: session_id.to_string(),
            schedule_id: None,
            current_total_tokens: Some(billed),
            current_input_tokens: Some(input),
            current_output_tokens: Some(output),
            billed_total_tokens: Some(i64::from(billed)),
            input_tokens: Some(input),
            output_tokens: Some(output),
            model_id: Some(SOL.to_string()),
            provider: Some("versa_azure".to_string()),
            cache_read_tokens: Some(cached),
            cache_creation_tokens: Some(0),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// A 300k-prompt Sol request (200k fresh + 100k cached) bills $8 / $0.80 /
    /// $30 per million; a 200k one bills the base $4 / $20.
    const LONG_USD: f64 = (200_000.0 * 8.0 + 100_000.0 * 0.80 + 2_000.0 * 30.0) / 1e6;
    const SHORT_USD: f64 = (200_000.0 * 4.0 + 2_000.0 * 20.0) / 1e6;

    #[tokio::test]
    async fn every_priced_total_bills_the_long_context_tier_per_request() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "sol".into(), SessionType::User)
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "long", 200_000, 100_000, 2_000))
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "short", 200_000, 0, 2_000))
            .await
            .unwrap();
        let want = LONG_USD + SHORT_USD;

        let snapshot = sm.spend_snapshot(&chat.id, true).await.unwrap();
        assert!(close(snapshot.chat_usd.unwrap(), want), "{snapshot:?}");
        assert!(snapshot.chat_usd_complete, "cache is priced now");
        assert!(close(snapshot.month_usd.unwrap(), want), "{snapshot:?}");
        assert!(snapshot.month_usd_complete);

        let summary = sm.get_usage_summary().await.unwrap();
        for totals in [&summary.month_to_date, &summary.all_time] {
            assert!(close(totals.cost.unwrap(), want), "{totals:?}");
            assert!(!totals.cost_excludes_cache);
            assert_eq!(totals.turns, 2, "two bands, still two turns");
        }

        for group in [UsageGroup::Model, UsageGroup::Day, UsageGroup::DayModel] {
            let report = sm.get_usage_report(0, i64::MAX, group).await.unwrap();
            assert_eq!(
                report.len(),
                1,
                "{group:?}: the bands roll up into one bucket"
            );
            assert!(
                close(report[0].cost.unwrap(), want),
                "{group:?}: {report:?}"
            );
            assert_eq!(report[0].turns, 2);
            assert_eq!(report[0].input_tokens, 400_000);
        }

        let rows = sm.get_session_model_usage(&chat.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(close(rows[0].cost.unwrap(), want), "{rows:?}");
        assert!(!rows[0].cost_is_partial);
        assert_eq!(rows[0].long_context_turns, 1);
        assert_eq!(rows[0].turns, 2);
    }

    /// The last request is the chat's own last turn request: a side call
    /// after it, a subagent's request and another chat's are not it.
    #[tokio::test]
    async fn the_last_request_is_the_chats_own_last_turn_request() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "sol".into(), SessionType::User)
            .await
            .unwrap();
        let other = sm
            .create_session(temp.path().to_path_buf(), "other".into(), SessionType::User)
            .await
            .unwrap();
        assert_eq!(sm.last_request_usage(&chat.id).await.unwrap(), None);

        sm.apply_usage_event(sol_turn(&chat.id, "t1", 1_000, 0, 10))
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "t2", 200, 30_000, 40))
            .await
            .unwrap();
        sm.apply_side_usage_event(super::super::SideUsageEntry {
            event_key: "side:session_name:1".into(),
            session_id: chat.id.clone(),
            provider: "versa_azure".into(),
            model_id: SOL.into(),
            input_tokens: Some(77),
            output_tokens: Some(40),
        })
        .await
        .unwrap();
        sm.apply_usage_event(sol_turn(&other.id, "o1", 5, 5, 5))
            .await
            .unwrap();

        let last = sm.last_request_usage(&chat.id).await.unwrap().unwrap();
        assert_eq!(last.input_tokens, 200);
        assert_eq!(last.cache_read_tokens, 30_000);
        assert_eq!(last.output_tokens, 40);
        assert_eq!(last.prompt_tokens(), 30_200);

        let any = sm
            .last_request_usage_for_model("VERSA_AZURE", SOL)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(any.input_tokens, 5, "the newest request to that model");
        assert_eq!(
            sm.last_request_usage_for_model("versa_azure", "another-model")
                .await
                .unwrap(),
            None
        );
    }

    /// A compaction after the chat's last turn request leaves no request to
    /// estimate from: its row (the whole old history in, the summary out) is
    /// not the next request's shape, and neither is the turn request before
    /// it, which carried the history the compaction replaced. The next turn
    /// request is the basis again.
    #[tokio::test]
    async fn a_compaction_is_never_the_basis_of_the_next_request() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "sol".into(), SessionType::User)
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "t1", 1_000, 249_000, 40))
            .await
            .unwrap();
        let key = super::compaction_event_key();
        assert!(key.starts_with(super::COMPACTION_EVENT_KEY_PREFIX), "{key}");
        sm.apply_usage_event(sol_turn(&chat.id, &key, 250_000, 0, 3_000))
            .await
            .unwrap();
        // A second summarization request of the same compaction.
        sm.apply_usage_event(sol_turn(&chat.id, &format!("{key}:1"), 250_000, 0, 2_000))
            .await
            .unwrap();
        assert_eq!(
            sm.last_request_usage(&chat.id).await.unwrap(),
            None,
            "no basis until a request after the compaction is billed"
        );
        // The model's cache shares are not read off a compaction either.
        let model = sm
            .last_request_usage_for_model("versa_azure", SOL)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(model.cache_read_tokens, 249_000, "{model:?}");
        // Still billed spend.
        let snapshot = sm.spend_snapshot(&chat.id, false).await.unwrap();
        assert_eq!(snapshot.chat_tokens, 250_040 + 253_000 + 252_000);

        sm.apply_usage_event(sol_turn(&chat.id, "t2", 2_000, 8_000, 50))
            .await
            .unwrap();
        let last = sm.last_request_usage(&chat.id).await.unwrap().unwrap();
        assert_eq!(last.prompt_tokens(), 10_000, "{last:?}");
    }

    /// The month read takes only this local month's rows; a row from an
    /// earlier month is not counted.
    #[tokio::test]
    async fn the_month_counts_this_months_rows_only() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "sol".into(), SessionType::User)
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "now", 1_000, 0, 10))
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "old", 7_000, 0, 10))
            .await
            .unwrap();
        // Forty days back is always an earlier calendar month.
        let pool = sm.storage.pool().await.unwrap();
        sqlx::query(
            "UPDATE token_events SET ts = CAST(strftime('%s', 'now', '-40 days') AS INTEGER)
              WHERE event_key = 'old'",
        )
        .execute(pool)
        .await
        .unwrap();

        let snapshot = sm.spend_snapshot(&chat.id, true).await.unwrap();
        assert_eq!(snapshot.month_tokens, 1_010, "{snapshot:?}");
        assert_eq!(snapshot.chat_tokens, 8_020, "a chat limit counts every row");
        let month_usd = (1_000.0 * 4.0 + 10.0 * 20.0) / 1e6;
        assert!(
            close(snapshot.month_usd.unwrap(), month_usd),
            "{snapshot:?}"
        );
    }

    async fn plan(pool: &sqlx::SqlitePool, sql: &str, bind_session: bool) -> Vec<String> {
        let explain = format!("EXPLAIN QUERY PLAN {sql}");
        let mut query = sqlx::query(&explain);
        if bind_session {
            query = query.bind("chat");
        }
        query
            .fetch_all(pool)
            .await
            .unwrap()
            .iter()
            .map(|row| sqlx::Row::get::<String, _>(row, "detail"))
            .collect()
    }

    /// `idx_sessions_parent` has no migration arm, so an existing database
    /// gains it when it is opened.
    #[tokio::test]
    async fn an_existing_database_gains_the_subagent_index_on_open() {
        let temp = TempDir::new().unwrap();
        let index_count = "SELECT COUNT(*) FROM sqlite_master \
                           WHERE type = 'index' AND name = 'idx_sessions_parent'";
        {
            let sm = SessionManager::new(temp.path().to_path_buf());
            let pool = sm.storage.pool().await.unwrap();
            let fresh: i64 = sqlx::query_scalar(index_count)
                .fetch_one(pool)
                .await
                .unwrap();
            assert_eq!(fresh, 1, "a fresh database has it");
            sqlx::query("DROP INDEX idx_sessions_parent")
                .execute(pool)
                .await
                .unwrap();
            pool.close().await;
        }
        let sm = SessionManager::new(temp.path().to_path_buf());
        let pool = sm.storage.pool().await.unwrap();
        let reopened: i64 = sqlx::query_scalar(index_count)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(reopened, 1, "an existing database gains it on open");
    }

    /// The spend read runs before every model request, so neither half scans
    /// the ledger: the month reads its rows through `idx_token_events_ts`, and
    /// a chat its own rows and its subagents' through `idx_token_events_session`
    /// and `idx_sessions_parent`.
    #[tokio::test]
    async fn the_spend_read_walks_the_ledger_indexes() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        sm.create_session(temp.path().to_path_buf(), "sol".into(), SessionType::User)
            .await
            .unwrap();
        let pool = sm.storage.pool().await.unwrap();

        for sql in [super::month_tokens_sql(), super::month_grain_sql()] {
            let steps = plan(pool, &sql, false).await;
            assert!(
                steps
                    .iter()
                    .any(|step| step.contains("USING INDEX idx_token_events_ts")),
                "the month no longer reads through idx_token_events_ts: {steps:?}"
            );
            assert!(
                !steps
                    .iter()
                    .any(|step| step.starts_with("SCAN te") || step == "SCAN token_events"),
                "the month scans the ledger: {steps:?}"
            );
        }

        for sql in [super::chat_tokens_sql(), super::chat_grain_sql()] {
            let steps = plan(pool, &sql, true).await;
            assert!(
                steps
                    .iter()
                    .any(|step| step.contains("idx_token_events_session")),
                "a chat's rows are not read through idx_token_events_session: {steps:?}"
            );
            assert!(
                steps
                    .iter()
                    .any(|step| step.contains("idx_sessions_parent")),
                "a chat's subagents are not found through idx_sessions_parent: {steps:?}"
            );
            assert!(
                !steps
                    .iter()
                    .any(|step| step.starts_with("SCAN te") || step.starts_with("SCAN sessions")),
                "the chat read scans a table: {steps:?}"
            );
        }
    }

    /// A deleted chat's usage is folded per day and model with no per-request
    /// size, so it reads the base rate from then on (documented on
    /// `BILLABLE_USAGE`). Cache stays priced.
    #[tokio::test]
    async fn a_deleted_chat_keeps_its_usage_at_the_base_rate() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "sol".into(), SessionType::User)
            .await
            .unwrap();
        sm.apply_usage_event(sol_turn(&chat.id, "long", 200_000, 100_000, 2_000))
            .await
            .unwrap();
        sm.delete_session(&chat.id).await.unwrap();

        let base = (200_000.0 * 4.0 + 100_000.0 * 0.40 + 2_000.0 * 20.0) / 1e6;
        let summary = sm.get_usage_summary().await.unwrap();
        assert!(close(summary.all_time.cost.unwrap(), base), "{summary:?}");
        assert!(!summary.all_time.cost_excludes_cache);
        assert_eq!(summary.all_time.turns, 1);
    }

    /// The SQL band and the Rust band agree on both sides of every threshold.
    #[tokio::test]
    async fn prompt_band_sql_matches_the_pricing_bands() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let sql = format!(
            "SELECT {} FROM (SELECT ?1 AS input_tokens, ?2 AS cache_read_tokens, \
             ?3 AS cache_creation_tokens)",
            prompt_band_sql!("")
        );
        let mut prompts = vec![0_i64, 1, 100_000];
        for threshold in LONG_CONTEXT_THRESHOLDS {
            prompts.extend([threshold - 1, threshold, threshold + 1]);
        }
        prompts.push(5_000_000);
        for prompt in prompts {
            // Split across the three buckets, and once with NULL buckets.
            let third = prompt / 3;
            let split: i64 = sqlx::query_scalar(&sql)
                .bind(third)
                .bind(third)
                .bind(prompt - 2 * third)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(split, prompt_band(prompt), "prompt {prompt}");
            let nulls: i64 = sqlx::query_scalar(&sql)
                .bind(prompt)
                .bind(None::<i64>)
                .bind(None::<i64>)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(nulls, prompt_band(prompt), "prompt {prompt}");
        }
    }
}
