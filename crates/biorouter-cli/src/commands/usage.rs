//! `biorouter usage` — per-day / per-model token + cost usage reports and a
//! month-to-date summary against the configured (display-only) monthly budget.
//!
//! Cost is computed through the same shared
//! [`biorouter::providers::pricing::model_cost`] helper the server and GUI use,
//! so the CLI can never disagree with the app on what a turn cost. Unknown
//! models show a `—` cost, never a misleading `$0`.

use anyhow::Result;
use biorouter::config::{percent_of, UsageLimits};
use biorouter::providers::decision::catalog::{baseten_display_name, BASETEN_DECISION_PROVIDER};
use biorouter::session::session_manager::DecisionUsageSubtotal;
use biorouter::session::{SessionManager, UsageGroup, UsageReportRow, UsageSummary};
use chrono::{Local, NaiveDate, TimeZone};

/// Parse a `YYYY-MM-DD` date to the unix second of its local `start`-of-day
/// (`false`) or last second of the day (`true`).
fn parse_local_date(date: &str, end_of_day: bool) -> Result<i64> {
    let day = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("invalid date '{date}', expected YYYY-MM-DD"))?;
    let time = if end_of_day {
        day.and_hms_opt(23, 59, 59).unwrap()
    } else {
        day.and_hms_opt(0, 0, 0).unwrap()
    };
    // A skipped/ambiguous local wall-clock instant (DST) resolves to the
    // earliest valid one; good enough for a day-granularity report boundary.
    Ok(Local
        .from_local_datetime(&time)
        .earliest()
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| Local.from_utc_datetime(&time).timestamp()))
}

pub async fn handle_usage(
    from: Option<String>,
    to: Option<String>,
    by_model: bool,
    json: bool,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let to_ts = match to {
        Some(d) => parse_local_date(&d, true)?,
        None => now,
    };
    let from_ts = match from {
        Some(d) => parse_local_date(&d, false)?,
        None => to_ts - 30 * 86_400,
    };
    let group = if by_model {
        UsageGroup::Model
    } else {
        UsageGroup::Day
    };

    let manager = SessionManager::instance();
    let rows = manager.get_usage_report(from_ts, to_ts, group).await?;
    let summary = manager.get_usage_summary().await?;
    let limits = UsageLimits::load();

    if json {
        println!("{}", usage_json(&rows, group, &summary, &limits)?);
    } else {
        print!(
            "{}",
            format_usage_text(&rows, group, &summary, &limits, from_ts, to_ts)
        );
    }
    Ok(())
}

/// Machine-readable output: the report rows plus the summary and configured
/// limits with their percents, mirroring the two HTTP routes.
fn usage_json(
    rows: &[UsageReportRow],
    group: UsageGroup,
    summary: &UsageSummary,
    limits: &UsageLimits,
) -> Result<String> {
    let token_percent = summary.month_to_date.total_tokens.and_then(|tokens| {
        percent_of(
            tokens as f64,
            limits.monthly_token_limit.map(|limit| limit as f64),
        )
    });
    let dollar_percent =
        if summary.month_to_date.has_unpriced || summary.month_to_date.cost_excludes_cache {
            None
        } else {
            summary
                .month_to_date
                .cost
                .and_then(|cost| percent_of(cost, limits.monthly_dollar_limit))
        };

    let value = serde_json::json!({
        "report": {
            "group": group,
            "rows": rows,
        },
        "summary": summary,
        "monthlyTokenLimit": limits.monthly_token_limit,
        "monthlyDollarLimit": limits.monthly_dollar_limit,
        "tokenPercent": token_percent,
        "dollarPercent": dollar_percent,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn fmt_tokens(n: i64) -> String {
    // Thousands separators, e.g. 13_300_000 -> "13,300,000".
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn fmt_optional_tokens(tokens: Option<i64>) -> String {
    tokens.map_or_else(|| "—".to_string(), fmt_tokens)
}

fn fmt_cost(cost: Option<f64>) -> String {
    match cost {
        // Sub-cent but non-zero: show enough precision to not read as $0.00.
        Some(c) if c > 0.0 && c < 0.01 => format!("${c:.4}"),
        Some(c) => format!("${c:.2}"),
        None => "—".to_string(),
    }
}

fn fmt_cost_with_completeness(cost: Option<f64>, partial: bool) -> String {
    let formatted = fmt_cost(cost);
    if partial && cost.is_some() {
        format!("≥{formatted}")
    } else {
        formatted
    }
}

/// Whether a decision subtotal is all requests to a local model with no
/// meter: something unmetered, nothing billed at an unknown price, and no
/// dollars.
fn decisions_unmetered_only(decisions: &DecisionUsageSubtotal) -> bool {
    decisions.has_unmetered && !decisions.has_unpriced && decisions.cost.unwrap_or(0.0) == 0.0
}

/// A row's COST cell. A bucket of requests to a local decision model alone
/// is counted as $0 by the report (the model has no meter), and reads "not
/// metered", as its decision line does. An unpriced Baseten bucket sets
/// `has_unpriced` and keeps the usual "—": Baseten bills it.
fn row_cost_cell(row: &UsageReportRow) -> String {
    let unmetered = row.turns == 0
        && !row.has_unpriced
        && row.decisions.as_ref().is_some_and(decisions_unmetered_only);
    if unmetered {
        return "not metered".to_string();
    }
    fmt_cost_with_completeness(row.cost, row.has_unpriced || row.cost_excludes_cache)
}

fn model_label(row: &UsageReportRow) -> String {
    if row.decisions.is_some() && row.turns == 0 {
        let model = row.model_id.as_deref().unwrap_or("unknown");
        let name = if row.provider.as_deref() == Some(BASETEN_DECISION_PROVIDER) {
            baseten_display_name(model)
        } else {
            model.to_string()
        };
        return format!("{name} (decision model)");
    }
    match (row.model_id.as_deref(), row.provider.as_deref()) {
        (Some(model), Some(provider)) => format!("{model} ({provider})"),
        (Some(model), None) => model.to_string(),
        _ => "unknown".to_string(),
    }
}

/// The decision-model share of several report rows together. The rows of one
/// grouping partition the range, so their sum is the range's.
fn sum_decisions(rows: &[UsageReportRow]) -> (i64, Option<DecisionUsageSubtotal>) {
    let requests = rows.iter().map(|row| row.decision_requests).sum();
    let subtotal = rows
        .iter()
        .filter_map(|row| row.decisions.as_ref())
        .cloned()
        .reduce(|a, b| DecisionUsageSubtotal {
            input_tokens: a.input_tokens + b.input_tokens,
            output_tokens: a.output_tokens + b.output_tokens,
            total_tokens: a.total_tokens.zip(b.total_tokens).map(|(a, b)| a + b),
            cost: match (a.cost, b.cost) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
            },
            has_unpriced: a.has_unpriced || b.has_unpriced,
            has_unmetered: a.has_unmetered || b.has_unmetered,
        });
    (requests, subtotal)
}

/// `Decision models: 600 requests, 1,200,600 tokens, $0.12`. A decision
/// model on Ollama has no meter, so its requests read "not metered". A
/// Baseten model the price list does not know is billed at a price nobody
/// here knows, so its requests read "unavailable", never "not metered".
fn decision_line(requests: i64, decisions: Option<&DecisionUsageSubtotal>) -> Option<String> {
    let decisions = decisions?;
    let tokens = decisions
        .total_tokens
        .unwrap_or(decisions.input_tokens + decisions.output_tokens);
    let cost = if decisions.has_unpriced {
        match decisions.cost {
            Some(_) => format!("{}, some unavailable", fmt_cost(decisions.cost)),
            None => "cost unavailable".to_string(),
        }
    } else if decisions_unmetered_only(decisions) || decisions.cost.is_none() {
        "not metered".to_string()
    } else if decisions.has_unmetered {
        format!("{}, some not metered", fmt_cost(decisions.cost))
    } else {
        fmt_cost(decisions.cost)
    };
    let noun = if requests == 1 { "request" } else { "requests" };
    Some(format!(
        "Decision models: {} {noun}, {} tokens, {cost}",
        fmt_tokens(requests),
        fmt_tokens(tokens),
    ))
}

fn format_report_rows(rows: &[UsageReportRow], group: UsageGroup) -> String {
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("  (no usage in this range)\n");
        return out;
    }

    let label_header = if matches!(group, UsageGroup::Model) {
        "MODEL"
    } else {
        "DAY"
    };
    out.push_str(&format!(
        "  {label_header:<28} {:>14} {:>14} {:>14} {:>14} {:>7} {:>12}\n",
        "INPUT", "OUTPUT", "CACHE", "TOTAL", "TURNS", "COST"
    ));
    let mut any_unpriced = false;
    let mut any_cost_excludes_cache = false;
    for row in rows {
        let label = match group {
            UsageGroup::Model => model_label(row),
            _ => row.date.clone().unwrap_or_else(|| "unknown".to_string()),
        };
        any_unpriced |= row.has_unpriced;
        any_cost_excludes_cache |= row.cost_excludes_cache;
        let cache_tokens = row
            .cache_read_tokens
            .zip(row.cache_creation_tokens)
            .map(|(read, creation)| read + creation);
        // A decision model's bucket has requests, not turns.
        let turns = if row.decisions.is_some() && row.turns == 0 {
            "—".to_string()
        } else {
            row.turns.to_string()
        };
        out.push_str(&format!(
            "  {label:<28} {:>14} {:>14} {:>14} {:>14} {:>7} {:>12}\n",
            fmt_tokens(row.input_tokens),
            fmt_tokens(row.output_tokens),
            fmt_optional_tokens(cache_tokens),
            fmt_optional_tokens(row.total_tokens),
            turns,
            row_cost_cell(row),
        ));
    }
    let (requests, decisions) = sum_decisions(rows);
    if let Some(line) = decision_line(requests, decisions.as_ref()) {
        out.push_str(&format!(
            "  {line}. Counted in TOTAL and COST, not in TURNS.\n"
        ));
    }
    if any_unpriced {
        out.push_str(
            "  Note: some rows include unpriced or incomplete usage; shown costs are known subtotals.\n",
        );
    }
    if any_cost_excludes_cache {
        out.push_str(
            "  Note: some priced models have no cache pricing; their cache tokens are excluded from cost.\n",
        );
    }
    out
}

fn format_usage_summary(summary: &UsageSummary, limits: &UsageLimits) -> String {
    let mut out = format!("Month to date ({})\n", summary.month);
    let mtd = &summary.month_to_date;
    out.push_str(&format!(
        "  Tokens: {}   Cost: {}   Turns: {}\n",
        fmt_optional_tokens(mtd.total_tokens),
        fmt_cost_with_completeness(mtd.cost, mtd.has_unpriced || mtd.cost_excludes_cache),
        mtd.turns,
    ));
    if let Some(line) = decision_line(mtd.decision_requests, mtd.decisions.as_ref()) {
        out.push_str(&format!("  {line}\n"));
    }
    let mtd_cache = mtd
        .cache_read_tokens
        .zip(mtd.cache_creation_tokens)
        .map(|(read, creation)| read + creation);
    if mtd_cache.is_some_and(|tokens| tokens > 0) {
        out.push_str(&format!(
            "  Cache: {} read, {} write\n",
            fmt_optional_tokens(mtd.cache_read_tokens),
            fmt_optional_tokens(mtd.cache_creation_tokens),
        ));
    } else if mtd_cache.is_none() {
        out.push_str("  Cache: — (incomplete historical accounting)\n");
    }
    if let Some(limit) = limits.monthly_token_limit {
        match mtd
            .total_tokens
            .and_then(|tokens| percent_of(tokens as f64, Some(limit as f64)))
        {
            Some(pct) => out.push_str(&format!(
                "  Token budget: {} / {}  ({pct:.1}%)\n",
                fmt_optional_tokens(mtd.total_tokens),
                fmt_tokens(limit),
            )),
            None => out.push_str(&format!(
                "  Token budget: — / {}  (billed total incomplete)\n",
                fmt_tokens(limit),
            )),
        }
    }
    if let Some(limit) = limits.monthly_dollar_limit {
        let exact_cost = (!mtd.has_unpriced && !mtd.cost_excludes_cache)
            .then_some(mtd.cost)
            .flatten();
        match exact_cost.and_then(|cost| percent_of(cost, Some(limit))) {
            Some(pct) => out.push_str(&format!(
                "  Dollar budget: {} / ${:.2}  ({pct:.1}%)\n",
                fmt_cost(mtd.cost),
                limit,
            )),
            None => out.push_str(&format!(
                "  Dollar budget: — / ${limit:.2}  (cost unavailable or incomplete)\n"
            )),
        }
    }

    out
}

/// Human-readable report + summary. Pure so the formatting is unit-tested
/// without a database.
fn format_usage_text(
    rows: &[UsageReportRow],
    group: UsageGroup,
    summary: &UsageSummary,
    limits: &UsageLimits,
    from_ts: i64,
    to_ts: i64,
) -> String {
    let format_day = |timestamp| {
        Local
            .timestamp_opt(timestamp, 0)
            .single()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    };
    let heading = match group {
        UsageGroup::Model => "Usage by model",
        _ => "Usage by day",
    };
    let mut out = format!(
        "{heading}  ({} → {})\n",
        format_day(from_ts),
        format_day(to_ts)
    );
    out.push_str(&format_report_rows(rows, group));
    out.push('\n');
    out.push_str(&format_usage_summary(summary, limits));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::session::UsageTotals;

    fn row(
        label: &str,
        is_model: bool,
        tokens: (i64, i64, i64),
        turns: i64,
        cost: Option<f64>,
        has_unpriced: bool,
    ) -> UsageReportRow {
        let (input, output, total) = tokens;
        UsageReportRow {
            date: (!is_model).then(|| label.to_string()),
            model_id: is_model.then(|| label.to_string()),
            provider: is_model.then(|| "zai".to_string()),
            input_tokens: input,
            output_tokens: output,
            total_tokens: Some(total),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns,
            cost,
            has_unpriced,
            cost_excludes_cache: false,
            decision_requests: 0,
            decisions: None,
        }
    }

    fn summary(total: i64, cost: Option<f64>) -> UsageSummary {
        let totals = || UsageTotals {
            input_tokens: total,
            output_tokens: 0,
            total_tokens: Some(total),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 3,
            cost,
            has_unpriced: cost.is_none(),
            cost_excludes_cache: false,
            decision_requests: 0,
            decisions: None,
        };
        UsageSummary {
            month: "2026-07".to_string(),
            month_to_date: totals(),
            all_time: totals(),
        }
    }

    #[test]
    fn tokens_get_thousands_separators() {
        assert_eq!(fmt_tokens(13_300_000), "13,300,000");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_000), "1,000");
        assert_eq!(fmt_tokens(0), "0");
    }

    #[test]
    fn cost_formatting_never_reads_zero_for_unknown() {
        assert_eq!(fmt_cost(None), "—");
        assert_eq!(fmt_cost(Some(5.0)), "$5.00");
        assert_eq!(fmt_cost(Some(1234.5)), "$1234.50");
        // Sub-cent shows extra precision rather than $0.00.
        assert_eq!(fmt_cost(Some(0.0034)), "$0.0034");
    }

    #[test]
    fn text_report_by_day_shows_rows_and_budget() {
        let rows = vec![
            row(
                "2026-07-10",
                false,
                (1_000_000, 0, 1_000_000),
                5,
                Some(1.40),
                true,
            ),
            row(
                "2026-07-11",
                false,
                (2_000_000, 1_000_000, 3_000_000),
                3,
                Some(7.20),
                false,
            ),
        ];
        let limits = UsageLimits {
            monthly_token_limit: Some(66_000_000),
            monthly_dollar_limit: Some(250.0),
            ..UsageLimits::default()
        };
        let text = format_usage_text(
            &rows,
            UsageGroup::Day,
            &summary(33_000_000, Some(8.60)),
            &limits,
            0,
            0,
        );
        assert!(text.contains("Usage by day"), "{text}");
        assert!(text.contains("2026-07-10"));
        assert!(text.contains("1,000,000"));
        assert!(text.contains("$1.40"));
        assert!(text.contains("$7.20"));
        // MTD budget: 33M / 66M = 50%.
        assert!(text.contains("50.0%"), "{text}");
        assert!(text.contains("Month to date (2026-07)"));
        // At least one row is partially unpriced → the note appears.
        assert!(text.contains("known subtotals"), "{text}");
    }

    #[test]
    fn text_report_by_model_flags_unknown_cost() {
        let rows = vec![
            row(
                "glm-5.2",
                true,
                (3_000_000, 1_000_000, 4_000_000),
                8,
                Some(8.60),
                false,
            ),
            // The unknown bucket: null cost, must render as —.
            UsageReportRow {
                date: None,
                model_id: None,
                provider: None,
                input_tokens: 500,
                output_tokens: 500,
                total_tokens: Some(1_000),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                turns: 2,
                cost: None,
                has_unpriced: true,
                cost_excludes_cache: false,
                decision_requests: 0,
                decisions: None,
            },
        ];
        let text = format_usage_text(
            &rows,
            UsageGroup::Model,
            &summary(4_001_000, None),
            &UsageLimits::default(),
            0,
            0,
        );
        assert!(text.contains("Usage by model"), "{text}");
        assert!(text.contains("glm-5.2 (zai)"));
        assert!(text.contains("unknown"));
        assert!(text.contains("—"), "null cost shows an em dash: {text}");
        // No configured limits → no budget lines.
        assert!(!text.contains("Token budget"));
        assert!(!text.contains("Dollar budget"));
    }

    #[test]
    fn dollar_budget_reports_unavailable_when_cost_unknown() {
        let limits = UsageLimits {
            monthly_token_limit: None,
            monthly_dollar_limit: Some(100.0),
            ..UsageLimits::default()
        };
        let text = format_usage_text(&[], UsageGroup::Day, &summary(500, None), &limits, 0, 0);
        assert!(text.contains("cost unavailable"), "{text}");
        assert!(text.contains("(no usage in this range)"));
    }

    #[test]
    fn json_output_carries_report_and_percents() {
        let rows = vec![row(
            "2026-07-10",
            false,
            (1_000_000, 0, 1_000_000),
            5,
            Some(1.40),
            false,
        )];
        let limits = UsageLimits {
            monthly_token_limit: Some(66_000_000),
            monthly_dollar_limit: Some(250.0),
            ..UsageLimits::default()
        };
        let json = usage_json(
            &rows,
            UsageGroup::Day,
            &summary(33_000_000, Some(125.0)),
            &limits,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["report"]["group"], "day");
        assert_eq!(value["report"]["rows"][0]["totalTokens"], 1_000_000);
        assert_eq!(value["monthlyTokenLimit"], 66_000_000);
        // 33M / 66M = 50%, $125 / $250 = 50%.
        assert_eq!(value["tokenPercent"], 50.0);
        assert_eq!(value["dollarPercent"], 50.0);
    }

    #[test]
    fn json_dollar_percent_null_when_cost_unknown() {
        let limits = UsageLimits {
            monthly_token_limit: Some(1_000),
            monthly_dollar_limit: Some(100.0),
            ..UsageLimits::default()
        };
        let json = usage_json(&[], UsageGroup::Day, &summary(500, None), &limits).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["tokenPercent"], 50.0);
        assert!(value["dollarPercent"].is_null());
    }

    #[test]
    fn text_report_shows_cache_column_and_excluded_note() {
        let rows = vec![UsageReportRow {
            date: Some("2026-07-12".to_string()),
            model_id: Some("claude-sonnet-4".to_string()),
            provider: Some("anthropic".to_string()),
            input_tokens: 1_000,
            output_tokens: 200,
            total_tokens: Some(1_800),
            cache_read_tokens: Some(500),
            cache_creation_tokens: Some(100),
            turns: 1,
            cost: Some(0.01),
            has_unpriced: false,
            cost_excludes_cache: true,
            decision_requests: 0,
            decisions: None,
        }];
        let text = format_usage_text(
            &rows,
            UsageGroup::Day,
            &summary(1_800, Some(0.01)),
            &UsageLimits::default(),
            0,
            0,
        );
        assert!(text.contains("CACHE"), "header has a CACHE column: {text}");
        // cache_read + cache_creation = 600 rendered in the row.
        assert!(text.contains("600"), "{text}");
        assert!(
            text.contains("no cache pricing"),
            "the cost-excludes-cache note appears: {text}"
        );
    }

    fn decision_row(
        provider: &str,
        model: &str,
        requests: i64,
        cost: Option<f64>,
    ) -> UsageReportRow {
        UsageReportRow {
            date: None,
            model_id: Some(model.to_string()),
            provider: Some(provider.to_string()),
            input_tokens: 1_200_000,
            output_tokens: 600,
            total_tokens: Some(1_200_600),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 0,
            cost,
            has_unpriced: false,
            cost_excludes_cache: false,
            decision_requests: requests,
            decisions: Some(DecisionUsageSubtotal {
                input_tokens: 1_200_000,
                output_tokens: 600,
                total_tokens: Some(1_200_600),
                cost,
                // An unpriced Ollama request has no meter; an unpriced Baseten
                // one is billed at a price nobody here knows.
                has_unpriced: cost.is_none() && provider != "ollama_decision",
                has_unmetered: cost.is_none() && provider == "ollama_decision",
            }),
        }
    }

    /// Decision requests are their own line: counted in tokens and dollars,
    /// not in turns, and a local decision model reads "not metered".
    #[test]
    fn text_report_shows_decision_models_on_their_own_line() {
        let rows = vec![
            row(
                "glm-5.2",
                true,
                (1_000_000, 0, 1_000_000),
                3,
                Some(1.40),
                false,
            ),
            decision_row(
                "baseten_decision",
                "decision-models/baseten-mammoth-27b-latest",
                600,
                Some(0.12),
            ),
        ];
        let mut month = summary(2_200_600, Some(1.52));
        month.month_to_date.decision_requests = 600;
        month.month_to_date.decisions = rows[1].decisions.clone();
        let text = format_usage_text(
            &rows,
            UsageGroup::Model,
            &month,
            &UsageLimits::default(),
            0,
            0,
        );
        assert!(text.contains("mammoth-1 (decision model)"), "{text}");
        assert!(
            text.contains("Decision models: 600 requests, 1,200,600 tokens, $0.12"),
            "{text}"
        );
        assert!(text.contains("not in TURNS"), "{text}");
        // The month summary carries the same line under its totals.
        let summary_text = format_usage_summary(&month, &UsageLimits::default());
        assert!(
            summary_text.contains("Decision models: 600 requests"),
            "{summary_text}"
        );
        let mammoth_line = text
            .lines()
            .find(|line| line.contains("mammoth-1"))
            .unwrap();
        assert!(mammoth_line.contains('—'), "no turns: {mammoth_line}");

        let local = [decision_row("ollama_decision", "tev1:0.8b", 1, None)];
        let (requests, decisions) = sum_decisions(&local);
        // The report counts a local model's bucket as $0; its row says why.
        let local_row = UsageReportRow {
            cost: Some(0.0),
            ..local[0].clone()
        };
        assert_eq!(row_cost_cell(&local_row), "not metered");
        let unlisted = UsageReportRow {
            cost: None,
            has_unpriced: true,
            ..decision_row("baseten_decision", "decision-models/x", 1, None)
        };
        assert_eq!(row_cost_cell(&unlisted), "—", "billed at an unknown rate");
        assert_eq!(
            decision_line(requests, decisions.as_ref()).unwrap(),
            "Decision models: 1 request, 1,200,600 tokens, not metered"
        );
        // A Baseten model the price list does not know is billed,
        // so its requests never read "not metered".
        let unpriced = [decision_row(
            "baseten_decision",
            "decision-models/baseten-unlisted-99b-latest",
            20,
            None,
        )];
        let (requests, decisions) = sum_decisions(&unpriced);
        let line = decision_line(requests, decisions.as_ref()).unwrap();
        assert_eq!(
            line,
            "Decision models: 20 requests, 1,200,600 tokens, cost unavailable"
        );
        // Mixed with a priced Baseten model, the dollars are a lower bound.
        let priced = decision_row(
            "baseten_decision",
            "decision-models/baseten-mammoth-27b-latest",
            600,
            Some(0.12),
        );
        let mixed = [priced.clone(), unpriced[0].clone()];
        let (requests, decisions) = sum_decisions(&mixed);
        assert!(
            decision_line(requests, decisions.as_ref())
                .unwrap()
                .ends_with(", $0.12, some unavailable"),
            "{decisions:?}"
        );
        // Mixed with a local model, the rest has no meter.
        let with_local = [priced, local[0].clone()];
        let (requests, decisions) = sum_decisions(&with_local);
        assert!(
            decision_line(requests, decisions.as_ref())
                .unwrap()
                .ends_with(", $0.12, some not metered"),
            "{decisions:?}"
        );
        // A local model alone reads "not metered" whether the subtotal's
        // cost is absent or a $0.
        let zero = DecisionUsageSubtotal {
            cost: Some(0.0),
            ..local[0].decisions.clone().unwrap()
        };
        assert!(decision_line(1, Some(&zero))
            .unwrap()
            .ends_with(", not metered"));
        let local_zero_row = UsageReportRow {
            cost: Some(0.0),
            decisions: Some(zero),
            ..local[0].clone()
        };
        assert_eq!(row_cost_cell(&local_zero_row), "not metered");

        // A range with no decision requests prints no line.
        let none = format_usage_text(
            &rows[..1],
            UsageGroup::Model,
            &summary(1_000_000, Some(1.40)),
            &UsageLimits::default(),
            0,
            0,
        );
        assert!(!none.contains("Decision models"), "{none}");
    }

    #[test]
    fn parse_local_date_rejects_garbage() {
        assert!(parse_local_date("not-a-date", false).is_err());
        assert!(parse_local_date("2026-07-10", false).is_ok());
        // End-of-day is strictly later than start-of-day for the same date.
        let start = parse_local_date("2026-07-10", false).unwrap();
        let end = parse_local_date("2026-07-10", true).unwrap();
        assert!(end > start);
        assert_eq!(end - start, 86_399);
    }
}
