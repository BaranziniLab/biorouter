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
//! summary, and are `None` when nothing in the span is priced.

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

/// The chat's own ledger rows plus its direct subagents', in the shared grain
/// shape so they price exactly like the usage summary.
fn chat_grain_sql() -> String {
    "SELECT '' AS day,
            te.model_id,
            te.provider,
            COALESCE(SUM(te.input_tokens), 0)  AS input_tokens,
            COALESCE(SUM(te.output_tokens), 0) AS output_tokens,
            CASE WHEN COUNT(te.billed_total_tokens) = COUNT(*)
                 THEN SUM(te.billed_total_tokens) END AS total_tokens,
            CASE WHEN COUNT(te.cache_read_tokens) = COUNT(*)
                 THEN SUM(te.cache_read_tokens) END AS cache_read_tokens,
            CASE WHEN COUNT(te.cache_creation_tokens) = COUNT(*)
                 THEN SUM(te.cache_creation_tokens) END AS cache_creation_tokens,
            COUNT(*) AS turns,
            CAST(COUNT(te.input_tokens) = COUNT(*) AS INTEGER) AS input_complete,
            CAST(COUNT(te.output_tokens) = COUNT(*) AS INTEGER) AS output_complete,
            CAST(COUNT(te.cache_read_tokens) = COUNT(*) AS INTEGER) AS cache_read_complete,
            CAST(COUNT(te.cache_creation_tokens) = COUNT(*) AS INTEGER) AS cache_creation_complete
       FROM token_events te
      WHERE te.session_id = ?1
         OR te.session_id IN (SELECT id FROM sessions WHERE parent_session_id = ?1)
      GROUP BY te.model_id, te.provider"
        .to_string()
}

impl SessionManager {
    /// Spend of `session_id` (with its subagents) and of the current month.
    /// `with_usd` prices both; leave it `false` when only token limits are set,
    /// since pricing may consult the network-backed catalog.
    pub async fn spend_snapshot(&self, session_id: &str, with_usd: bool) -> Result<SpendSnapshot> {
        self.storage.spend_snapshot(session_id, with_usd).await
    }
}

impl SessionStorage {
    async fn spend_snapshot(&self, session_id: &str, with_usd: bool) -> Result<SpendSnapshot> {
        let pool = self.pool().await?;

        let chat_tokens: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(COALESCE(billed_total_tokens,
                                          COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))), 0)
               FROM token_events
              WHERE session_id = ?1
                 OR session_id IN (SELECT id FROM sessions WHERE parent_session_id = ?1)",
        )
        .bind(session_id)
        .fetch_one(pool)
        .await?;

        let month_tokens: i64 = sqlx::query_scalar(&format!(
            "SELECT COALESCE(SUM(COALESCE(u.billed_total_tokens,
                                          COALESCE(u.input_tokens, 0) + COALESCE(u.output_tokens, 0))), 0)
               FROM {BILLABLE_USAGE}
              WHERE substr(u.day, 1, 7) = strftime('%Y-%m', 'now', 'localtime')"
        ))
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

        let month_grain = sqlx::query_as::<_, UsageGrainRow>(&super::billable_usage_grain_sql(
            "''",
            "substr(u.day, 1, 7) = strftime('%Y-%m', 'now', 'localtime')",
        ))
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
