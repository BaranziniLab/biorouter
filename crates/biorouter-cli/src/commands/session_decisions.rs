//! `biorouter session decisions <SESSION_ID>`: every decision-model call one
//! chat made, aggregated first and then paged.
//!
//! Decision calls can number in the thousands in one chat (a monitor every
//! iteration, a `decide` batch over hundreds of chunks), so the output leads
//! with the aggregates every surface draws: one line per source and criterion,
//! then one line per source per recent turn. Individual calls follow as a page,
//! newest first, with filters and a cursor for the next page. This is the
//! terminal counterpart of the desktop's decision history view, which is the
//! only other place individual calls are listed.
//!
//! The data comes from the same functions every surface reads,
//! `biorouter::decision::log::{summary, list}`:
//!
//! * **With a reachable daemon** this asks `GET /sessions/{id}/decisions/summary`
//!   and `GET /sessions/{id}/decisions`, which apply the daemon's reach gate.
//! * **With none** it runs the same functions against the local session store,
//!   under the same tier rule the route applies, so unsetting the secret is not
//!   a way around the daemon's gate: a terminal on a public model is refused a
//!   private chat.

use anyhow::{anyhow, bail, Result};
use biorouter::decision::log::{
    DecisionAggregate, DecisionPage, DecisionQuery, DecisionRecord, DecisionTurnMarkerItem,
    DecisionTurnSummary, SessionDecisionSummary, MAX_PAGE,
};
use biorouter::decision::{DecisionOrigin, DecisionOutcome};
use biorouter::privacy::visibility::may_read;
use biorouter::privacy::ProviderTier;
use biorouter::session::SessionManager;
use chrono::{DateTime, Local, Utc};

use super::apps::{configured_port, daemon_ok, DAEMON_HOST};
use super::decide::{clip, format_latency, format_p, origin_word, probability_bar, short_model};
use super::session_watch::{
    daemon_auth, daemon_get_json, no_daemon_at, refusal_sentence, with_supplied_key, DaemonAuth,
};

/// How long one request to the daemon may take.
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// The page size when `--limit` is not given.
pub const DEFAULT_LIMIT: u32 = 20;
/// Cells of the short bar beside each line.
const SHORT_BAR: usize = 8;

/// What a terminal on a public model is told about a private chat on the local
/// path. The daemon's own refusal is printed as the daemon wrote it.
pub const LOCAL_OUT_OF_REACH: &str =
    "This chat is private, and this terminal is configured for a model hosted outside your \
     institution, so its decisions are not shown here. Run this from a terminal configured for a \
     private model, or open the chat in the desktop app.";

/// `--source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SourceArg {
    Monitor,
    Tool,
    Gate,
    Chat,
    Adhoc,
}

impl From<SourceArg> for DecisionOrigin {
    fn from(source: SourceArg) -> Self {
        match source {
            SourceArg::Monitor => DecisionOrigin::Monitor,
            SourceArg::Tool => DecisionOrigin::Tool,
            SourceArg::Gate => DecisionOrigin::Gate,
            SourceArg::Chat => DecisionOrigin::Chat,
            SourceArg::Adhoc => DecisionOrigin::Adhoc,
        }
    }
}

/// `--outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutcomeArg {
    Passed,
    Failed,
    Decided,
    Error,
    Refused,
}

impl From<OutcomeArg> for DecisionOutcome {
    fn from(outcome: OutcomeArg) -> Self {
        match outcome {
            OutcomeArg::Passed => DecisionOutcome::Passed,
            OutcomeArg::Failed => DecisionOutcome::Failed,
            OutcomeArg::Decided => DecisionOutcome::Decided,
            OutcomeArg::Error => DecisionOutcome::Error,
            OutcomeArg::Refused => DecisionOutcome::Refused,
        }
    }
}

/// The filters `session decisions` takes.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct DecisionFilters {
    /// Only calls from this source.
    #[arg(long, value_enum)]
    pub source: Option<SourceArg>,
    /// Only calls from this monitor, tool call or phase (its id).
    #[arg(long = "origin-id", value_name = "ID")]
    pub origin_id: Option<String>,
    /// Only calls made in this turn.
    #[arg(long, value_name = "TURN_ID")]
    pub turn: Option<String>,
    /// Only calls of this `decide` batch.
    #[arg(long, value_name = "BATCH_ID")]
    pub batch: Option<String>,
    /// Only calls with this outcome.
    #[arg(long, value_enum)]
    pub outcome: Option<OutcomeArg>,
    /// Only calls whose lowest pass probability is at least this.
    #[arg(long = "min-p", value_name = "P", value_parser = super::decide::parse_probability)]
    pub min_p: Option<f64>,
    /// Only calls whose lowest pass probability is at most this.
    #[arg(long = "max-p", value_name = "P", value_parser = super::decide::parse_probability)]
    pub max_p: Option<f64>,
    /// Only calls that failed with an error or were refused.
    #[arg(long)]
    pub errors: bool,
    /// Calls per page (at most 200).
    #[arg(short, long, value_name = "N", default_value_t = DEFAULT_LIMIT)]
    pub limit: u32,
    /// Continue after a previous page: the cursor its last line printed.
    #[arg(long, value_name = "CURSOR")]
    pub before: Option<String>,
}

impl DecisionFilters {
    pub fn query(&self) -> DecisionQuery {
        DecisionQuery {
            decision_id: None,
            origin: self.source.map(Into::into),
            origin_id: self.origin_id.clone(),
            turn_id: self.turn.clone(),
            batch_id: self.batch.clone(),
            outcome: self.outcome.map(Into::into),
            min_p: self.min_p,
            max_p: self.max_p,
            errors_only: self.errors,
            before: self.before.clone(),
            limit: Some(self.limit.clamp(1, MAX_PAGE)),
        }
    }

    /// Whether any filter narrows the list.
    fn narrows(&self) -> bool {
        self.source.is_some()
            || self.origin_id.is_some()
            || self.turn.is_some()
            || self.batch.is_some()
            || self.outcome.is_some()
            || self.min_p.is_some()
            || self.max_p.is_some()
            || self.errors
    }
}

/// The query string `GET /sessions/{id}/decisions` takes: the query's fields in
/// camelCase, as the route deserializes them.
pub fn query_string(query: &DecisionQuery) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    let enum_word = |value: serde_json::Value| value.as_str().map(str::to_string);
    if let Some(origin) = query.origin {
        if let Some(word) = serde_json::to_value(origin).ok().and_then(enum_word) {
            pairs.push(("origin", word));
        }
    }
    if let Some(id) = &query.origin_id {
        pairs.push(("originId", id.clone()));
    }
    if let Some(id) = &query.turn_id {
        pairs.push(("turnId", id.clone()));
    }
    if let Some(id) = &query.batch_id {
        pairs.push(("batchId", id.clone()));
    }
    if let Some(outcome) = query.outcome {
        if let Some(word) = serde_json::to_value(outcome).ok().and_then(enum_word) {
            pairs.push(("outcome", word));
        }
    }
    if let Some(p) = query.min_p {
        pairs.push(("minP", p.to_string()));
    }
    if let Some(p) = query.max_p {
        pairs.push(("maxP", p.to_string()));
    }
    if query.errors_only {
        pairs.push(("errorsOnly", "true".to_string()));
    }
    if let Some(cursor) = &query.before {
        pairs.push(("before", cursor.clone()));
    }
    if let Some(limit) = query.limit {
        pairs.push(("limit", limit.to_string()));
    }
    if pairs.is_empty() {
        return String::new();
    }
    let joined = pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("?{joined}")
}

// ── pure renderers ──────────────────────────────────────────────────────────

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What one origin's calls are called: a monitor and a phase gate check,
/// everything else calls.
fn calls_word(origin: DecisionOrigin, count: u64) -> String {
    match origin {
        DecisionOrigin::Monitor | DecisionOrigin::Gate => plural(count, "check", "checks"),
        _ => plural(count, "call", "calls"),
    }
}

/// `["18 checks", "18 failed", "2 errors"]`, leaving out what is zero.
pub fn count_parts(
    origin: DecisionOrigin,
    count: u64,
    passed: u64,
    failed: u64,
    errors: u64,
) -> Vec<String> {
    let mut parts = vec![calls_word(origin, count)];
    if passed > 0 {
        parts.push(format!("{passed} passed"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if errors > 0 {
        parts.push(plural(errors, "error", "errors"));
    }
    parts
}

/// `18 checks · 18 failed · 2 errors`, leaving out what is zero.
pub fn counts_text(
    origin: DecisionOrigin,
    count: u64,
    passed: u64,
    failed: u64,
    errors: u64,
) -> String {
    count_parts(origin, count, passed, failed, errors).join(" · ")
}

/// What one origin is called in a marker line.
fn origin_title(origin: DecisionOrigin) -> &'static str {
    match origin {
        DecisionOrigin::Monitor => "Monitor",
        DecisionOrigin::Tool => "Decide",
        DecisionOrigin::Gate => "Gate",
        DecisionOrigin::Chat => "Decision chat",
        DecisionOrigin::Adhoc => "Decision",
    }
}

/// The text of one origin's marker line: `Monitor Form reached: 18 checks this
/// turn · 18 failed · last P 0.31`. A monitor that fired gets ` · fired` after
/// it, drawn apart.
pub fn marker_text(item: &DecisionTurnMarkerItem) -> String {
    let mut parts = count_parts(
        item.origin,
        item.count,
        item.passed,
        item.failed,
        item.errors,
    );
    if let Some(first) = parts.first_mut() {
        first.push_str(" this turn");
    }
    let title = origin_title(item.origin);
    let label = clip(&item.label, 40);
    // The decide tool's label is usually its own name: say it once.
    let named = if label.is_empty() || label.eq_ignore_ascii_case(title) {
        title.to_string()
    } else {
        format!("{title} {label}")
    };
    let mut text = format!("{named}: {}", parts.join(" · "));
    if let Some(p) = item.last_pass_probability {
        text.push_str(&format!(" · last P {}", format_p(p)));
    }
    text
}

/// One aggregate as a line: source, label, counts, the last answer and its
/// probability with a short bar, and the model.
pub fn aggregate_line(aggregate: &DecisionAggregate, label_w: usize) -> String {
    let mut line = format!(
        "  {:<7}  {:<label_w$}  {}",
        origin_word(aggregate.origin),
        clip(&aggregate.label, 32),
        counts_text(
            aggregate.origin,
            aggregate.count,
            aggregate.passed,
            aggregate.failed,
            aggregate.errors
        ),
    );
    if let Some(answer) = &aggregate.last_answer {
        line.push_str(&format!(" · last {}", clip(answer, 20)));
    }
    if let Some(p) = aggregate.last_pass_probability {
        line.push_str(&format!(
            " P {} {}",
            format_p(p),
            probability_bar(p, SHORT_BAR)
        ));
    }
    if let Some(model) = &aggregate.model {
        line.push_str(&format!(" · {}", short_model(model)));
    }
    line
}

/// One turn's aggregate as a line.
pub fn turn_line(turn: &DecisionTurnSummary, label_w: usize) -> String {
    let mut line = format!(
        "  {}  {:<7}  {:<label_w$}  {}",
        short_id(&turn.turn_id),
        origin_word(turn.origin),
        clip(&turn.label, 32),
        counts_text(
            turn.origin,
            turn.count,
            turn.passed,
            turn.failed,
            turn.errors
        ),
    );
    if let Some(p) = turn.last_pass_probability {
        line.push_str(&format!(" · last P {}", format_p(p)));
    }
    line.push_str(&format!(
        " · {} to {}",
        clock(turn.first_at),
        clock(turn.last_at)
    ));
    line
}

/// The first eight characters of an id, enough to tell turns apart on screen.
pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M:%S").to_string()
}

fn when(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local)
        .format("%m-%d %H:%M:%S")
        .to_string()
}

/// The probability a list row shows: the lowest pass probability when a
/// question had a threshold, else the first answer's own probability.
pub fn record_probability(record: &DecisionRecord) -> Option<f64> {
    let pass = record
        .decisions
        .iter()
        .filter_map(|d| d.pass_probability)
        .fold(None, |low: Option<f64>, p| {
            Some(low.map_or(p, |l| l.min(p)))
        });
    pass.or_else(|| {
        record
            .decisions
            .first()
            .map(super::decide::answer_probability)
    })
}

/// `passed`, `failed`, `decided`, `error`, `refused`.
pub fn outcome_word(outcome: DecisionOutcome) -> &'static str {
    match outcome {
        DecisionOutcome::Passed => "passed",
        DecisionOutcome::Failed => "failed",
        DecisionOutcome::Decided => "decided",
        DecisionOutcome::Error => "error",
        DecisionOutcome::Refused => "refused",
    }
}

/// One call as a line: when, source, label (and chunk), the answers, p with a
/// short bar, outcome, model and latency; an error's sentence instead of the
/// answers.
pub fn record_line(record: &DecisionRecord) -> String {
    let label = record
        .label
        .clone()
        .or_else(|| record.origin_id.clone())
        .unwrap_or_else(|| origin_word(record.origin).to_string());
    let mut head = format!(
        "  {}  {:<7}  {}",
        when(record.created_at),
        origin_word(record.origin),
        clip(&label, 28)
    );
    if let Some(chunk) = &record.chunk_id {
        head.push_str(&format!(" [{}]", clip(chunk, 24)));
    }
    if let Some(error) = &record.error {
        return format!(
            "{head}  {}: {}",
            outcome_word(record.outcome),
            clip(error, 120)
        );
    }
    let answers = record
        .decisions
        .iter()
        .map(|d| {
            if record.decisions.len() > 1 {
                format!("{} {}", d.question, super::decide::answer_label(d))
            } else {
                super::decide::answer_label(d)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut line = format!("{head}  {}", clip(&answers, 48));
    if let Some(p) = record_probability(record) {
        line.push_str(&format!(
            "  P {} {}",
            format_p(p),
            probability_bar(p, SHORT_BAR)
        ));
    }
    line.push_str(&format!("  {}", outcome_word(record.outcome)));
    if record.calibrated == Some(false) {
        line.push_str(" (uncalibrated)");
    }
    if let Some(model) = &record.model {
        line.push_str(&format!("  {}", short_model(model)));
    }
    if let Some(ms) = record.latency_ms {
        line.push_str(&format!("  {}", format_latency(ms)));
    }
    line
}

/// The whole text output: aggregates, recent turns, then the page.
pub fn render_text(
    session_id: &str,
    summary: &SessionDecisionSummary,
    page: &DecisionPage,
    filters: &DecisionFilters,
) -> Vec<String> {
    if summary.total == 0 {
        return vec![format!(
            "Session {session_id} has made no decision-model calls."
        )];
    }
    let mut out = vec![format!(
        "Decisions in session {session_id}: {}",
        plural(summary.total, "call", "calls")
    )];
    let label_w = summary
        .aggregates
        .iter()
        .map(|a| clip(&a.label, 32).chars().count())
        .chain(
            summary
                .recent_turns
                .iter()
                .map(|t| clip(&t.label, 32).chars().count()),
        )
        .max()
        .unwrap_or(0);
    if !summary.aggregates.is_empty() {
        out.push(String::new());
        out.push("By source:".to_string());
        out.extend(
            summary
                .aggregates
                .iter()
                .map(|aggregate| aggregate_line(aggregate, label_w)),
        );
    }
    if !summary.recent_turns.is_empty() {
        out.push(String::new());
        out.push("Recent turns:".to_string());
        out.extend(
            summary
                .recent_turns
                .iter()
                .map(|turn| turn_line(turn, label_w)),
        );
    }
    out.push(String::new());
    let filtered = if filters.narrows() { " matching" } else { "" };
    if page.records.is_empty() {
        out.push(format!("No{filtered} calls on this page."));
    } else {
        out.push(format!(
            "Calls{filtered}, newest first ({} of {}):",
            page.records.len(),
            page.total
        ));
        out.extend(page.records.iter().map(record_line));
    }
    if let Some(cursor) = &page.next_cursor {
        out.push(format!(
            "More: biorouter session decisions {session_id} --before {cursor}"
        ));
    }
    out
}

// ── reading ─────────────────────────────────────────────────────────────────

/// The route's JSON body, read from the first `{` to the last `}`.
pub fn parse_json<T: serde::de::DeserializeOwned>(body: &str, what: &str) -> Result<T> {
    let slice = body
        .find('{')
        .zip(body.rfind('}'))
        .and_then(|(start, end)| body.get(start..=end))
        .ok_or_else(|| anyhow!("the daemon's answer held no JSON object"))?;
    serde_json::from_str(slice)
        .map_err(|error| anyhow!("the daemon's answer was not {what}: {error}"))
}

/// A refusal or failure from the daemon as an error. A daemon that answers and
/// refuses is never a reason to fall back to the local store.
pub fn daemon_error(session_id: &str, path: &str, code: u16, body: &str) -> anyhow::Error {
    match code {
        404 => anyhow!("Session '{session_id}' not found."),
        403 => anyhow!(
            "{}",
            refusal_sentence(body).unwrap_or_else(|| "the daemon refused the request".into())
        ),
        401 => anyhow!(
            "the daemon refused the secret key (401): BIOROUTER_SERVER__SECRET_KEY does not match \
             the daemon's"
        ),
        other => anyhow!(
            "the daemon answered {path} with {other}: {}",
            refusal_sentence(body).unwrap_or_default()
        ),
    }
}

async fn get_from_daemon<T: serde::de::DeserializeOwned>(
    session_id: &str,
    path: &str,
    auth: &DaemonAuth,
    what: &str,
) -> Result<T> {
    let (code, body) = daemon_get_json(path, auth, configured_port(), REQUEST_DEADLINE).await?;
    if code != 200 {
        return Err(daemon_error(session_id, path, code, &body));
    }
    parse_json(&body, what)
}

async fn from_daemon(
    session_id: &str,
    query: &DecisionQuery,
    auth: &DaemonAuth,
) -> Result<(SessionDecisionSummary, DecisionPage)> {
    let summary = get_from_daemon(
        session_id,
        &format!("/sessions/{session_id}/decisions/summary"),
        auth,
        "a decision summary",
    )
    .await?;
    let page = get_from_daemon(
        session_id,
        &format!("/sessions/{session_id}/decisions{}", query_string(query)),
        auth,
        "a page of decisions",
    )
    .await?;
    Ok((summary, page))
}

/// The local path's reach rule, the one the route applies: a private chat is
/// shown only to a terminal whose capability may read it.
pub async fn check_local_reach(
    manager: &SessionManager,
    session_id: &str,
    capability: ProviderTier,
    enforced: bool,
    refusal: &str,
) -> Result<()> {
    let session = manager
        .get_session(session_id, false)
        .await
        .map_err(|_| anyhow!("Session '{session_id}' not found."))?;
    if enforced && !may_read(capability, session.privacy_tier) {
        bail!("{refusal}");
    }
    Ok(())
}

async fn from_store(
    manager: &SessionManager,
    session_id: &str,
    query: &DecisionQuery,
    capability: ProviderTier,
    enforced: bool,
) -> Result<(SessionDecisionSummary, DecisionPage)> {
    check_local_reach(
        manager,
        session_id,
        capability,
        enforced,
        LOCAL_OUT_OF_REACH,
    )
    .await?;
    let summary = biorouter::decision::log::summary(manager, session_id).await?;
    let page = biorouter::decision::log::list(manager, session_id, query).await?;
    Ok((summary, page))
}

/// Where the next read should go: the daemon, with its credentials, or the
/// local store, with the reason no daemon answered.
pub(crate) enum Reach {
    Daemon(DaemonAuth),
    Local(String),
}

/// Decide where a session command reads from. The user-action key is read
/// from stdin only once a daemon is known to be listening, because it is only
/// ever sent to one.
pub(crate) async fn reach(user_action_key_stdin: bool) -> Result<Reach> {
    let auth = match daemon_auth().await {
        Ok(auth) => auth,
        Err(_) => {
            return Ok(Reach::Local(
                "BIOROUTER_SERVER__SECRET_KEY is not set, so no daemon was asked".to_string(),
            ))
        }
    };
    let port = configured_port();
    if !daemon_ok(DAEMON_HOST, port).await {
        return Ok(Reach::Local(no_daemon_at(port)));
    }
    Ok(Reach::Daemon(
        with_supplied_key(auth, user_action_key_stdin).await?,
    ))
}

/// `biorouter session decisions <SESSION_ID> ...`.
pub async fn handle_session_decisions(
    session_id: &str,
    filters: DecisionFilters,
    json: bool,
    user_action_key_stdin: bool,
) -> Result<()> {
    let query = filters.query();
    let (summary, page) = match reach(user_action_key_stdin).await? {
        Reach::Daemon(auth) => from_daemon(session_id, &query, &auth).await?,
        Reach::Local(why) => {
            eprintln!("note: read from the local session store ({why}).");
            let manager = SessionManager::instance();
            let enforced = biorouter::privacy::privacy_tiers_enabled();
            let capability = crate::session::privacy::caller_capability().await;
            from_store(&manager, session_id, &query, capability, enforced).await?
        }
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "summary": summary,
                "page": page,
            }))?
        );
        return Ok(());
    }
    for line in render_text(session_id, &summary, &page, &filters) {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::commands::decide::tests::yes_no;
    use biorouter::decision::DecisionModelRef;

    pub(crate) fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + seconds, 0).unwrap()
    }

    pub(crate) fn aggregate(origin: DecisionOrigin, label: &str, count: u64) -> DecisionAggregate {
        DecisionAggregate {
            origin,
            origin_id: Some(format!("{label}-id")),
            label: label.to_string(),
            count,
            passed: 0,
            failed: count,
            errors: 0,
            last_at: at(60),
            last_answer: Some("no".into()),
            last_pass_probability: Some(0.31),
            model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
        }
    }

    pub(crate) fn record(id: &str, p_yes: f64) -> DecisionRecord {
        let mut decision = yes_no(p_yes);
        decision.pass_probability = Some(p_yes);
        decision.passed = Some(p_yes >= 0.85);
        DecisionRecord {
            id: id.to_string(),
            session_id: Some("s1".into()),
            created_at: at(30),
            origin: DecisionOrigin::Monitor,
            origin_id: Some("form-reached".into()),
            label: Some("Form reached".into()),
            turn_id: Some("0193a1b2c3d4".into()),
            batch_id: None,
            chunk_index: None,
            chunk_id: None,
            model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            served_model: None,
            tier: Some("private".into()),
            calibrated: Some(true),
            questions: serde_json::json!({}),
            decisions: vec![decision],
            outcome: if p_yes >= 0.85 {
                DecisionOutcome::Passed
            } else {
                DecisionOutcome::Failed
            },
            context_preview: String::new(),
            context_bytes: 1200,
            context_truncated: false,
            error: None,
            latency_ms: Some(22),
            input_tokens: Some(300),
            output_tokens: Some(1),
            privacy_enforced: true,
        }
    }

    #[test]
    fn the_filters_become_the_routes_camel_case_query() {
        let filters = DecisionFilters {
            source: Some(SourceArg::Monitor),
            origin_id: Some("form reached".into()),
            turn: Some("t1".into()),
            outcome: Some(OutcomeArg::Failed),
            min_p: Some(0.4),
            max_p: Some(0.6),
            errors: true,
            limit: 500,
            before: Some("c/2".into()),
            ..Default::default()
        };
        let query = filters.query();
        assert_eq!(query.limit, Some(MAX_PAGE), "the page is capped");
        assert_eq!(
            query_string(&query),
            "?origin=monitor&originId=form%20reached&turnId=t1&outcome=failed&minP=0.4&maxP=0.6\
             &errorsOnly=true&before=c%2F2&limit=200"
        );
        assert_eq!(
            query_string(
                &DecisionFilters {
                    limit: DEFAULT_LIMIT,
                    ..Default::default()
                }
                .query()
            ),
            "?limit=20"
        );
    }

    #[test]
    fn aggregates_come_first_then_turns_then_the_page() {
        let mut tool = aggregate(DecisionOrigin::Tool, "decide", 600);
        tool.passed = 41;
        tool.failed = 557;
        tool.errors = 2;
        tool.last_answer = Some("yes".into());
        tool.last_pass_probability = Some(0.93);
        let summary = SessionDecisionSummary {
            total: 618,
            aggregates: vec![aggregate(DecisionOrigin::Monitor, "Form reached", 18), tool],
            recent_turns: vec![DecisionTurnSummary {
                turn_id: "0193a1b2c3d4e5".into(),
                origin: DecisionOrigin::Monitor,
                origin_id: Some("form-reached".into()),
                label: "Form reached".into(),
                count: 18,
                passed: 0,
                failed: 18,
                errors: 0,
                first_at: at(0),
                last_at: at(60),
                last_pass_probability: Some(0.31),
            }],
        };
        let page = DecisionPage {
            records: vec![record("d2", 0.31), record("d1", 0.9)],
            next_cursor: Some("cur-1".into()),
            total: 618,
        };
        let lines = render_text("s1", &summary, &page, &DecisionFilters::default());
        assert_eq!(lines[0], "Decisions in session s1: 618 calls");
        let by_source = lines.iter().position(|l| l == "By source:").unwrap();
        let turns = lines.iter().position(|l| l == "Recent turns:").unwrap();
        let calls = lines
            .iter()
            .position(|l| l.starts_with("Calls, newest first (2 of 618):"))
            .unwrap();
        assert!(by_source < turns && turns < calls);
        let monitor = &lines[by_source + 1];
        assert!(monitor.contains("monitor  Form reached"), "{monitor}");
        assert!(
            monitor.contains("18 checks · 18 failed · last no P 0.31"),
            "{monitor}"
        );
        assert!(monitor.ends_with("tev1:0.8b"), "{monitor}");
        let tool = &lines[by_source + 2];
        assert!(
            tool.contains("600 calls · 41 passed · 557 failed · 2 errors · last yes P 0.93"),
            "{tool}"
        );
        assert!(
            lines[turns + 1].starts_with("  0193a1b2  monitor"),
            "{}",
            lines[turns + 1]
        );
        assert!(
            lines[calls + 1].contains("Form reached  no  P 0.31"),
            "{}",
            lines[calls + 1]
        );
        assert!(lines[calls + 1].contains("failed  tev1:0.8b  22 ms"));
        assert!(lines[calls + 2].contains("yes  P 0.90"));
        assert_eq!(
            lines.last().unwrap(),
            "More: biorouter session decisions s1 --before cur-1"
        );
    }

    /// Labels, chunk ids, option names and errors are a model's to choose (the
    /// `decide` tool), so none of their control characters reaches the
    /// terminal: no escape sequence, bell or C1 CSI survives into a line.
    #[test]
    fn model_written_text_prints_without_control_characters() {
        use biorouter::decision::types::Answer;
        let mut tool = aggregate(DecisionOrigin::Tool, "\x1b]0;owned\x07decide", 600);
        tool.last_answer = Some("\u{9b}2Jyes".into());
        let summary = SessionDecisionSummary {
            total: 602,
            aggregates: vec![tool],
            recent_turns: vec![DecisionTurnSummary {
                turn_id: "0193a1b2c3d4e5".into(),
                origin: DecisionOrigin::Gate,
                origin_id: Some("build".into()),
                label: "\x1b[31mBuild".into(),
                count: 2,
                passed: 1,
                failed: 1,
                errors: 0,
                first_at: at(0),
                last_at: at(60),
                last_pass_probability: Some(0.5),
            }],
        };
        let mut forged = record("d1", 0.9);
        forged.label = Some("\x1b[2J\x1b[Hall checks passed".into());
        forged.chunk_id = Some("\x1b]52;c;cm0gLXJmIH4K\x07".into());
        forged.decisions[0].answer = Answer::Choice {
            option: "\x1b]0;x\x07ms".into(),
        };
        forged.model = Some(DecisionModelRef::new("ollama_decision", "tev1\x1b[2J"));
        let mut broken = record("d2", 0.5);
        broken.decisions.clear();
        broken.error = Some("boom\u{9b}2J\x07".into());
        let page = DecisionPage {
            records: vec![forged, broken],
            next_cursor: None,
            total: 2,
        };
        let lines = render_text("s1", &summary, &page, &DecisionFilters::default());
        for line in &lines {
            assert!(!line.chars().any(char::is_control), "{line:?}");
        }
        let text = lines.join("\n");
        assert!(text.contains("all checks passed"), "{text}");
        assert!(text.contains("boom"), "{text}");
        assert!(
            !marker_text(&DecisionTurnMarkerItem {
                origin: DecisionOrigin::Gate,
                origin_id: Some("build".into()),
                label: "\x1b[2JBuild\x07".into(),
                count: 1,
                passed: 1,
                failed: 0,
                errors: 0,
                last_pass_probability: None,
                fired: false,
            })
            .chars()
            .any(char::is_control),
            "the transcript marker line"
        );
    }

    #[test]
    fn an_error_row_says_what_failed_and_an_empty_chat_says_so() {
        let mut failed = record("d3", 0.5);
        failed.decisions.clear();
        failed.outcome = DecisionOutcome::Refused;
        failed.error =
            Some("This chat is private, so a public decision model cannot read it.".into());
        failed.chunk_id = Some("note-0412".into());
        let line = record_line(&failed);
        assert!(line.contains("[note-0412]"), "{line}");
        assert!(line.contains("refused: This chat is private"), "{line}");

        let empty = SessionDecisionSummary {
            total: 0,
            aggregates: vec![],
            recent_turns: vec![],
        };
        let page = DecisionPage {
            records: vec![],
            next_cursor: None,
            total: 0,
        };
        assert_eq!(
            render_text("s9", &empty, &page, &DecisionFilters::default()),
            vec!["Session s9 has made no decision-model calls.".to_string()]
        );
    }

    #[test]
    fn the_row_probability_is_the_lowest_pass_probability() {
        let mut two = record("d4", 0.9);
        let mut second = yes_no(0.4);
        second.question = "edss_stated".into();
        second.pass_probability = Some(0.4);
        two.decisions.push(second);
        assert_eq!(record_probability(&two), Some(0.4));
        let line = record_line(&two);
        assert!(line.contains("has_ms yes, edss_stated no"), "{line}");

        let mut plain = record("d5", 0.7);
        plain.decisions[0].pass_probability = None;
        assert_eq!(record_probability(&plain), Some(0.7), "the answer's own p");
    }

    /// A private chat in a throwaway store, raised the way a turn on a private
    /// model raises it.
    pub(crate) async fn private_chat(
        manager: &SessionManager,
        dir: &std::path::Path,
    ) -> biorouter::session::Session {
        use biorouter::privacy::SessionClassification;
        let session = manager
            .create_session(
                dir.to_path_buf(),
                "private chat".to_string(),
                biorouter::session::SessionType::User,
            )
            .await
            .unwrap();
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(biorouter::model::ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        session
    }

    /// The local path end to end: rows the decision log's sink wrote come back
    /// as aggregates and a page, through the functions every surface reads.
    #[tokio::test]
    async fn rows_the_sink_wrote_read_back_as_aggregates_then_a_page() {
        use biorouter::decision::service::{DecisionLogEntry, DecisionSink};
        use biorouter::decision::DecisionContext;
        let dir = tempfile::tempdir().unwrap();
        let manager = std::sync::Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let chat = manager
            .create_session(
                dir.path().to_path_buf(),
                "notes".to_string(),
                biorouter::session::SessionType::User,
            )
            .await
            .unwrap();
        let sink = biorouter::decision::log::SqliteDecisionSink {
            session_manager: manager.clone(),
        };
        let entry = |origin: DecisionOrigin, origin_id: &str, label: &str, p: f64| {
            let mut decision = yes_no(p);
            decision.pass_probability = Some(p);
            decision.passed = Some(p >= 0.85);
            // A row is stored under its request's id, one per request.
            decision.request_id = format!("{origin_id}-{p}");
            DecisionLogEntry {
                session_id: Some(chat.id.clone()),
                origin,
                origin_id: Some(origin_id.to_string()),
                origin_label: Some(label.to_string()),
                turn_id: Some("turn-1".into()),
                batch_id: None,
                chunk_index: None,
                chunk_id: None,
                model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
                served_model: None,
                tier: Some(ProviderTier::Private),
                questions: std::iter::once((
                    "has_ms".to_string(),
                    biorouter::decision::Question {
                        instructions: "Is the form reached?".into(),
                        kind: biorouter::decision::DecisionKind::Binary {},
                        threshold: Some(biorouter::decision::Threshold::Yes(0.85)),
                    },
                ))
                .collect(),
                context: DecisionContext::Text("the window".into()),
                context_bytes: 10,
                outcome: if p >= 0.85 {
                    DecisionOutcome::Passed
                } else {
                    DecisionOutcome::Failed
                },
                decisions: vec![decision],
                error: None,
                latency_ms: Some(22),
                usage: Default::default(),
                privacy_enforced: true,
            }
        };
        for p in [0.2, 0.31] {
            sink.record(entry(
                DecisionOrigin::Monitor,
                "form-reached",
                "Form reached",
                p,
            ))
            .await
            .unwrap();
        }
        sink.record(entry(DecisionOrigin::Tool, "call_1", "decide", 0.93))
            .await
            .unwrap();

        let filters = DecisionFilters {
            limit: DEFAULT_LIMIT,
            ..Default::default()
        };
        let (summary, page) = from_store(
            &manager,
            &chat.id,
            &filters.query(),
            ProviderTier::Private,
            true,
        )
        .await
        .unwrap();
        assert_eq!(summary.total, 3);
        assert_eq!(page.total, 3);
        assert_eq!(page.records.len(), 3);
        let text = render_text(&chat.id, &summary, &page, &filters).join("\n");
        assert!(text.contains("Decisions in session"), "{text}");
        assert!(text.contains("monitor  Form reached"), "{text}");
        assert!(text.contains("2 checks"), "{text}");
        assert!(text.contains("tool"), "{text}");

        let monitors_only = DecisionFilters {
            source: Some(SourceArg::Monitor),
            limit: DEFAULT_LIMIT,
            ..Default::default()
        };
        let (_, page) = from_store(
            &manager,
            &chat.id,
            &monitors_only.query(),
            ProviderTier::Private,
            true,
        )
        .await
        .unwrap();
        assert_eq!(page.total, 2, "the source filter reaches the store");
    }

    #[tokio::test]
    async fn the_local_path_refuses_a_private_chat_to_a_public_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let session = private_chat(&manager, dir.path()).await;
        let query = DecisionFilters::default().query();
        let refused = from_store(&manager, &session.id, &query, ProviderTier::Public, true)
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), LOCAL_OUT_OF_REACH);
        // The switch off: shown (and announced elsewhere).
        assert!(
            from_store(&manager, &session.id, &query, ProviderTier::Public, false)
                .await
                .is_ok()
        );
        assert!(
            from_store(&manager, &session.id, &query, ProviderTier::Private, true)
                .await
                .is_ok()
        );
        assert!(
            from_store(&manager, "missing", &query, ProviderTier::Private, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
    }
}
