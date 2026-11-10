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

/// The chat's own ledger rows plus its direct subagents', in the shared grain
/// shape (prompt bands included) so they price exactly like the usage summary.
fn chat_grain_sql() -> String {
    super::token_events_grain_sql(
        "te.session_id = ?1
            OR te.session_id IN (SELECT id FROM sessions WHERE parent_session_id = ?1)",
    )
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
