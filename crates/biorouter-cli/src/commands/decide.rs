//! `biorouter decide` and `biorouter models decision list`: ask a decision model
//! one typed question from the terminal, and see which decision models this
//! machine can use.
//!
//! A decision model returns a typed answer (yes or no, one of named options, a
//! level of a score) with the probability of every outcome, instead of text.
//! The command is modelled on `ollama run --ask`: the state is read from stdin,
//! `--text`, or one file per decision with `--batch <dir>`.
//!
//! Every call goes through `biorouter::decision::global()`, the one service the
//! agent loop, the routes and the desktop use, so validation, the size check,
//! the deadline and Gate H are the same here as everywhere else.
//!
//! * **With `--session <id>`** the decision is about that chat: the service
//!   reads the chat's classification and refuses a model outside its tier
//!   before anything is sent, and the call is written to the chat's decision
//!   log with the `adhoc` origin.
//! * **Without one** nothing is logged and no tier is enforced, because there is
//!   no chat to protect. The text is the person's own; when the model the
//!   command resolved is hosted outside the institution, one line on stderr says
//!   so before anything is sent. The person is warned; the agent never is the
//!   one choosing here.
//!
//! The renderers below are pure functions over decision values, shared with
//! `session decisions` and the TUI, so every terminal surface draws a
//! probability the same way.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use biorouter::decision::types::validate_questions;
use biorouter::decision::{
    Answer, ChunkOutcome, Decision, DecisionChunk, DecisionContext, DecisionKind,
    DecisionModelEntry, DecisionModelRef, DecisionModelSource, DecisionOrigin, DecisionRequest,
    DecisionService, DecisionSession, ProbabilitySource, Question, Threshold,
};
use biorouter::privacy::ProviderTier;
use biorouter::session::SessionManager;

/// Cells a probability bar takes.
pub const BAR_WIDTH: usize = 24;
/// Files larger than this are reported in the batch summary, never read.
pub const MAX_BATCH_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// The service takes this many chunks per batch; a larger directory is sent in
/// groups of this size, one after another.
const BATCH_GROUP: usize = biorouter::decision::service::MAX_BATCH_CHUNKS;
/// The question's name when `--name` is not given.
pub const DEFAULT_QUESTION_NAME: &str = "question";
/// Widest an outcome label is printed beside its bar.
const LABEL_MAX: usize = 24;

/// What every text rendering says once about what the numbers are.
pub const PROBABILITY_NOTE: &str =
    "These are the model's own probabilities. They have not been validated on your data.";

/// `biorouter decide`.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct DecideOptions {
    /// The decision model, `provider/model` (see `biorouter models decision list`).
    #[arg(long, value_name = "PROVIDER/MODEL")]
    pub model: Option<String>,

    /// The question to ask about the text.
    #[arg(long, value_name = "QUESTION")]
    pub ask: String,

    /// A name for the question (letters, digits, `_` and `-`), shown in the log.
    #[arg(long, value_name = "NAME", default_value = DEFAULT_QUESTION_NAME)]
    pub name: String,

    /// One option of a choice question, `name` or `name=description`. Repeat it
    /// for each option (at least two).
    #[arg(
        long = "choice",
        value_name = "NAME[=DESCRIPTION]",
        value_parser = parse_choice,
        conflicts_with = "levels"
    )]
    pub choices: Vec<(String, Option<String>)>,

    /// Score levels, lowest first, separated by commas (1 to 10 levels).
    #[arg(long, value_name = "LEVEL,LEVEL,...", value_delimiter = ',')]
    pub levels: Option<Vec<String>>,

    /// Pass when the probability of the wanted answer is at least this (0 to 1).
    #[arg(long, value_name = "P", value_parser = parse_probability)]
    pub threshold: Option<f64>,

    /// The answer the threshold is on: `yes` or `no` for a yes/no question
    /// (default `yes`), an option name for a choice, or a level (its text or
    /// its number from 0) for a score, which then counts that level and every
    /// level above it.
    #[arg(long = "threshold-on", value_name = "ANSWER", requires = "threshold")]
    pub threshold_on: Option<String>,

    /// The text to decide on, instead of reading it from stdin.
    #[arg(long, value_name = "TEXT", conflicts_with = "batch")]
    pub text: Option<String>,

    /// Decide once per file in this directory (not recursive), several at a
    /// time, and print a summary table.
    #[arg(long, value_name = "DIR")]
    pub batch: Option<PathBuf>,

    /// Record the decision in this chat's decision log. The chat's privacy
    /// tier then decides which decision models may read the text.
    #[arg(long, value_name = "SESSION_ID")]
    pub session: Option<String>,

    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

/// `--choice name=description`.
pub fn parse_choice(raw: &str) -> Result<(String, Option<String>), String> {
    let (name, description) = match raw.split_once('=') {
        Some((name, description)) => (name.trim(), Some(description.trim())),
        None => (raw.trim(), None),
    };
    if name.is_empty() {
        return Err("a choice needs a name before `=`".to_string());
    }
    let description = description
        .filter(|d| !d.is_empty())
        .map(|d| strip_quotes(d).to_string());
    Ok((strip_quotes(name).to_string(), description))
}

fn strip_quotes(text: &str) -> &str {
    let text = text.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// `--threshold`: a probability.
pub fn parse_probability(raw: &str) -> Result<f64, String> {
    let p: f64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("`{raw}` is not a number"))?;
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(format!("{raw} is not a probability between 0 and 1"));
    }
    Ok(p)
}

/// The one question the options describe, checked the way the service checks
/// it, so a mistake is reported before anything is sent.
pub fn build_question(options: &DecideOptions) -> Result<Question> {
    let kind = if !options.choices.is_empty() {
        DecisionKind::Choice {
            options: options.choices.iter().cloned().collect(),
        }
    } else if let Some(levels) = &options.levels {
        DecisionKind::Score {
            levels: levels
                .iter()
                .map(|level| level.trim().to_string())
                .collect(),
        }
    } else {
        DecisionKind::Binary {}
    };
    let threshold = match options.threshold {
        None => None,
        Some(p) => Some(threshold_for(&kind, p, options.threshold_on.as_deref())?),
    };
    let question = Question {
        instructions: options.ask.trim().to_string(),
        kind,
        threshold,
    };
    let questions = std::iter::once((options.name.clone(), question.clone())).collect();
    validate_questions(&questions).map_err(|e| anyhow!("{e}"))?;
    Ok(question)
}

fn threshold_for(kind: &DecisionKind, p: f64, on: Option<&str>) -> Result<Threshold> {
    match kind {
        DecisionKind::Binary {} => match on.map(str::trim).map(str::to_ascii_lowercase) {
            None => Ok(Threshold::Yes(p)),
            Some(answer) if answer == "yes" => Ok(Threshold::Yes(p)),
            Some(answer) if answer == "no" => Ok(Threshold::No(p)),
            Some(answer) => {
                bail!("a yes/no question's threshold is on `yes` or `no`, not `{answer}`")
            }
        },
        DecisionKind::Choice { options } => {
            let Some(on) = on.map(str::trim) else {
                bail!(
                    "say which option the threshold is on with --threshold-on (one of: {})",
                    options.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            };
            Ok(Threshold::Option {
                name: on.to_string(),
                p,
            })
        }
        DecisionKind::Score { levels } => {
            let Some(on) = on.map(str::trim) else {
                bail!(
                    "say which level the threshold counts from with --threshold-on (a level's \
                     text, or its number from 0 to {})",
                    levels.len().saturating_sub(1)
                );
            };
            let level = levels
                .iter()
                .position(|level| level.eq_ignore_ascii_case(on))
                .or_else(|| on.parse::<usize>().ok())
                .ok_or_else(|| anyhow!("`{on}` is not one of the levels"))?;
            Ok(Threshold::AtLeastLevel { level, p })
        }
    }
}

// ── pure renderers, shared by every terminal surface ────────────────────────

/// A bar `width` cells wide, filled to `p` in eighths of a cell, on a light
/// track.
pub fn probability_bar(p: f64, width: usize) -> String {
    const PARTIAL: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
    let p = if p.is_finite() {
        p.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let eighths = (p * (width * 8) as f64).round() as usize;
    let full = eighths / 8;
    let rest = eighths % 8;
    let mut bar = "█".repeat(full);
    bar.push_str(PARTIAL[rest]);
    let drawn = full + usize::from(rest > 0);
    bar.push_str(&"░".repeat(width.saturating_sub(drawn)));
    bar
}

/// `0.83`.
pub fn format_p(p: f64) -> String {
    format!("{p:.2}")
}

/// The answer a decision points to, as a person reads it: `yes`, an option's
/// name, or a score level's text.
pub fn answer_label(decision: &Decision) -> String {
    match &decision.answer {
        Answer::Binary { value } => if *value { "yes" } else { "no" }.to_string(),
        // Option names and level texts may be a model's (the `decide` tool's).
        Answer::Choice { option } => terminal_text(option),
        Answer::Score { level, label, .. } => {
            if label.trim().is_empty() {
                format!("level {level}")
            } else {
                terminal_text(label)
            }
        }
    }
}

/// The key of [`Decision::probabilities`] the answer is: `yes` or `no`, the
/// option name, or the score level's index (never its text).
pub fn answer_key(decision: &Decision) -> String {
    match &decision.answer {
        Answer::Binary { value } => if *value { "yes" } else { "no" }.to_string(),
        Answer::Choice { option } => option.clone(),
        Answer::Score { level, .. } => level.to_string(),
    }
}

/// The probability of the answer itself.
pub fn answer_probability(decision: &Decision) -> f64 {
    decision
        .probabilities
        .get(&answer_key(decision))
        .copied()
        .unwrap_or(0.0)
}

/// `calibrated`, or `uncalibrated` with where the number came from.
pub fn calibration_flag(decision: &Decision) -> String {
    if decision.calibrated {
        return "calibrated".to_string();
    }
    match &decision.source {
        ProbabilitySource::Native => "uncalibrated".to_string(),
        ProbabilitySource::Logprob { mass } => {
            format!(
                "uncalibrated (logprob, {:.0}% on the options)",
                mass * 100.0
            )
        }
        ProbabilitySource::Verbalized { .. } => "uncalibrated (the model wrote it)".to_string(),
    }
}

/// The last segment of a model id: `tev1:0.8b`, `elephant-1`.
pub fn short_model(model: &DecisionModelRef) -> String {
    terminal_text(model.model.rsplit('/').next().unwrap_or(&model.model))
}

/// `1.2 s` or `22 ms`.
pub fn format_latency(ms: u64) -> String {
    if ms >= 1_000 {
        format!("{:.1} s", ms as f64 / 1_000.0)
    } else {
        format!("{ms} ms")
    }
}

/// The word every surface names an origin with.
pub fn origin_word(origin: DecisionOrigin) -> &'static str {
    match origin {
        DecisionOrigin::Chat => "chat",
        DecisionOrigin::Monitor => "monitor",
        DecisionOrigin::Tool => "tool",
        DecisionOrigin::Gate => "gate",
        DecisionOrigin::Adhoc => "adhoc",
    }
}

/// The readable name of each outcome key, in the question's order: `yes` and
/// `no`, the option names, or the score levels' texts.
pub fn outcome_labels(decision: &Decision, question: Option<&Question>) -> Vec<(String, String)> {
    decision
        .probabilities
        .keys()
        .map(|key| {
            let label = match question.map(|q| &q.kind) {
                Some(DecisionKind::Score { levels }) => key
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| levels.get(i).cloned())
                    .unwrap_or_else(|| format!("level {key}")),
                _ => key.clone(),
            };
            (key.clone(), label)
        })
        .collect()
}

/// Text a model or a stored row supplied, made safe to print: every control
/// character (an escape sequence, a bell, a carriage return, a C1 code such
/// as CSI) becomes a space, so a label, an option name, a chunk id or an error
/// cannot move the cursor, recolour or retitle the terminal, write the
/// clipboard or forge a line. The `decide` tool lets a model choose chunk ids
/// and option names freely.
pub fn terminal_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Clip `text` to `max` characters, with an ellipsis when cut, as
/// [`terminal_text`] with its whitespace collapsed.
pub fn clip(text: &str, max: usize) -> String {
    let text = terminal_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// What the threshold compared, as a sentence: `P(yes) 0.82 ≥ 0.80, passed`.
pub fn threshold_line(decision: &Decision, question: Option<&Question>) -> Option<String> {
    let threshold = question?.threshold.as_ref()?;
    let p = decision.pass_probability?;
    let (wanted, bound) = match threshold {
        Threshold::Yes(t) => ("P(yes)".to_string(), *t),
        Threshold::No(t) => ("P(no)".to_string(), *t),
        Threshold::Option { name, p } => (format!("P({name})"), *p),
        Threshold::AtLeastLevel { level, p } => {
            let label = match &question?.kind {
                DecisionKind::Score { levels } => levels.get(*level).cloned(),
                _ => None,
            }
            .unwrap_or_else(|| format!("level {level}"));
            (format!("P(at least {label})"), *p)
        }
    };
    let passed = decision.passed == Some(true);
    let sign = if passed { "≥" } else { "<" };
    Some(format!(
        "{wanted} {} {sign} {}, {}",
        format_p(p),
        format_p(bound),
        if passed { "passed" } else { "failed" }
    ))
}

/// One decision as text: the question, one bar per outcome with the answer
/// marked, then the answer, its probability, the calibration flag, the model
/// and the latency, then the threshold when there was one.
pub fn render_decision(decision: &Decision, question: Option<&Question>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(question) = question {
        out.push(question.instructions.trim().to_string());
    }
    let labels = outcome_labels(decision, question);
    let width = labels
        .iter()
        .map(|(_, label)| clip(label, LABEL_MAX).chars().count())
        .max()
        .unwrap_or(0);
    let answer = answer_key(decision);
    for (key, label) in &labels {
        let p = decision.probabilities.get(key).copied().unwrap_or(0.0);
        let marker = if *key == answer { "▸" } else { " " };
        out.push(format!(
            "{marker} {:<width$}  {}  {}",
            clip(label, LABEL_MAX),
            probability_bar(p, BAR_WIDTH),
            format_p(p)
        ));
    }
    out.push(format!(
        "Answer: {}  p {}  {}  {}  {}",
        answer_label(decision),
        format_p(answer_probability(decision)),
        calibration_flag(decision),
        decision.model.label(),
        format_latency(decision.latency_ms)
    ));
    if let Some(line) = threshold_line(decision, question) {
        out.push(format!("Threshold: {line}"));
    }
    out
}

/// One row of a batch: a file and what was decided about it, or why not.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchRow {
    pub file: String,
    pub decision: Option<Decision>,
    pub error: Option<String>,
}

impl BatchRow {
    fn from_outcome(outcome: ChunkOutcome) -> Self {
        Self {
            file: outcome.chunk_id,
            decision: outcome
                .decisions
                .and_then(|mut d| (!d.is_empty()).then(|| d.remove(0))),
            error: outcome.error,
        }
    }
}

/// What a batch came to, for its last line and for `--json`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub files: usize,
    pub decided: usize,
    pub errors: usize,
    /// How many files got each answer, most first, ties by name.
    pub answers: Vec<(String, usize)>,
    /// Set when the question had a threshold.
    pub passed: Option<usize>,
    pub uncalibrated: usize,
}

pub fn summarize_batch(rows: &[BatchRow]) -> BatchSummary {
    let mut answers: std::collections::BTreeMap<String, usize> = Default::default();
    let mut summary = BatchSummary {
        files: rows.len(),
        ..Default::default()
    };
    for row in rows {
        match &row.decision {
            Some(decision) => {
                summary.decided += 1;
                *answers.entry(answer_label(decision)).or_default() += 1;
                if decision.passed.is_some() {
                    *summary.passed.get_or_insert(0) += usize::from(decision.passed == Some(true));
                }
                summary.uncalibrated += usize::from(!decision.calibrated);
            }
            None => summary.errors += 1,
        }
    }
    let mut answers: Vec<(String, usize)> = answers.into_iter().collect();
    answers.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    summary.answers = answers;
    summary
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The summary line of a batch.
pub fn batch_summary_line(summary: &BatchSummary) -> String {
    let mut parts = vec![plural(summary.files, "file", "files")];
    for (answer, count) in &summary.answers {
        parts.push(format!("{answer} {count}"));
    }
    if let Some(passed) = summary.passed {
        parts.push(format!("{passed} passed the threshold"));
    }
    if summary.errors > 0 {
        parts.push(plural(summary.errors, "error", "errors"));
    }
    if summary.uncalibrated > 0 {
        parts.push(format!("{} uncalibrated", summary.uncalibrated));
    }
    format!("Summary: {}", parts.join(" · "))
}

/// A batch as a table: one row per file (answer, p, a short bar, outcome),
/// then the summary, then each error in full.
pub fn render_batch(rows: &[BatchRow], question: &Question) -> Vec<String> {
    let file_w = rows
        .iter()
        .map(|row| clip(&row.file, 40).chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let answer_w = rows
        .iter()
        .filter_map(|row| row.decision.as_ref())
        .map(|d| clip(&answer_label(d), 16).chars().count())
        .max()
        .unwrap_or(6)
        .max(6);
    let mut out = vec![
        clip(question.instructions.trim(), 100),
        format!(
            "  {:<file_w$}  {:<answer_w$}  {:<4}  {:<10}  outcome",
            "file", "answer", "p", ""
        ),
    ];
    for row in rows {
        let file = clip(&row.file, 40);
        match &row.decision {
            Some(decision) => {
                let p = answer_probability(decision);
                let outcome = match decision.passed {
                    Some(true) => "passed",
                    Some(false) => "failed",
                    None => "decided",
                };
                out.push(format!(
                    "  {file:<file_w$}  {:<answer_w$}  {}  {}  {outcome}",
                    clip(&answer_label(decision), 16),
                    format_p(p),
                    probability_bar(p, 10)
                ));
            }
            None => out.push(format!(
                "  {file:<file_w$}  {:<answer_w$}  {:<4}  {:<10}  error",
                "", "", ""
            )),
        }
    }
    let summary = summarize_batch(rows);
    out.push(batch_summary_line(&summary));
    let errors: Vec<&BatchRow> = rows.iter().filter(|row| row.error.is_some()).collect();
    if !errors.is_empty() {
        out.push("Errors:".to_string());
        for row in errors {
            out.push(format!(
                "  {}: {}",
                row.file,
                row.error.as_deref().unwrap_or_default()
            ));
        }
    }
    out
}

/// `public` or `private`.
pub fn tier_word(tier: ProviderTier) -> &'static str {
    match tier {
        ProviderTier::Public => "public",
        ProviderTier::Private => "private",
    }
}

/// `1,234`.
fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `biorouter models decision list` as text.
pub fn render_model_list(entries: &[DecisionModelEntry]) -> Vec<String> {
    if entries.is_empty() {
        return vec![NO_MODEL_HELP.to_string()];
    }
    let name_w = entries
        .iter()
        .map(|e| format!("{}/{}", e.provider, e.model).chars().count())
        .max()
        .unwrap_or(0);
    let mut out = vec![format!("Decision models ({}, local first):", entries.len())];
    for entry in entries {
        let kinds: Vec<&str> = entry
            .kinds
            .iter()
            .map(|kind| match kind {
                biorouter::decision::types::DecisionKindTag::Binary => "yes/no",
                biorouter::decision::types::DecisionKindTag::Choice => "choice",
                biorouter::decision::types::DecisionKindTag::Score => "score",
            })
            .collect();
        let place = if entry.local {
            format!("{} · this computer", tier_word(entry.tier))
        } else {
            tier_word(entry.tier).to_string()
        };
        let adapter = serde_json::to_value(entry.adapter)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        out.push(format!(
            "  {:<name_w$}  {place} · {} tokens · {} · {adapter}",
            format!("{}/{}", entry.provider, entry.model),
            group_thousands(entry.context_tokens),
            kinds.join(", ")
        ));
    }
    out
}

/// What is said when no decision model can be used.
pub const NO_MODEL_HELP: &str =
    "No decision model is available. Pull a decision model into a local Ollama (for example \
     `ollama pull tev1:0.8b`), add a Baseten API key, or set BIOROUTER_DECISION_MODEL.";

/// What a terminal on a public model is told when it names a private chat.
pub const SESSION_OUT_OF_REACH: &str =
    "This chat is private, and this terminal is configured for a model hosted outside your \
     institution, so a decision cannot be recorded in it from here. Run this from a terminal \
     configured for a private model, or leave out --session.";

/// The stderr line for a public model asked with no chat to protect.
pub fn public_model_warning(display: &str) -> String {
    format!(
        "warning: {display} is hosted outside your institution (public tier). Do not send it \
         private or patient data."
    )
}

// ── running it ──────────────────────────────────────────────────────────────

/// The model an ad hoc decision will use, resolved here (rather than inside
/// the service) so the person is warned about the model that will actually
/// read the text.
#[derive(Debug, Clone)]
struct ResolvedModel {
    model_ref: DecisionModelRef,
    display: String,
    tier: ProviderTier,
}

/// `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL`, read the way the
/// service reads them.
enum ConfiguredDefault {
    Model(DecisionModelRef),
    Provider(String),
}

fn configured_default() -> Option<ConfiguredDefault> {
    let config = biorouter::config::Config::global();
    let read = |key: &str| {
        config
            .get_param::<String>(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    match (
        read("BIOROUTER_DECISION_PROVIDER"),
        read("BIOROUTER_DECISION_MODEL"),
    ) {
        (Some(provider), Some(model)) => Some(ConfiguredDefault::Model(DecisionModelRef::new(
            provider, model,
        ))),
        (None, Some(model)) => DecisionModelRef::parse(&model).map(ConfiguredDefault::Model),
        (Some(provider), None) => Some(ConfiguredDefault::Provider(provider)),
        (None, None) => None,
    }
}

async fn resolve_adhoc(
    service: &DecisionService,
    wanted: Option<DecisionModelRef>,
) -> Result<ResolvedModel> {
    let entries = service.models().await;
    let from_entry = |entry: &DecisionModelEntry| ResolvedModel {
        model_ref: entry.model_ref(),
        display: entry.display_name.clone(),
        tier: entry.tier,
    };
    let wanted = match wanted {
        Some(model) => Some(model),
        None => match configured_default() {
            Some(ConfiguredDefault::Model(model)) => Some(model),
            Some(ConfiguredDefault::Provider(provider)) => {
                return entries
                    .iter()
                    .find(|entry| entry.provider == provider)
                    .map(from_entry)
                    .ok_or_else(|| {
                        anyhow!(
                            "No `{provider}` decision model is available. {}",
                            NO_MODEL_HELP
                        )
                    });
            }
            None => None,
        },
    };
    match wanted {
        Some(model) => {
            if let Some(entry) = entries.iter().find(|entry| entry.answers_to(&model)) {
                return Ok(from_entry(entry));
            }
            let opened = biorouter::providers::decision::ConfiguredDecisionModels
                .open(&model)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            Ok(ResolvedModel {
                model_ref: opened.model_ref(),
                display: opened.display_name(),
                tier: opened.tier(),
            })
        }
        None => entries
            .first()
            .map(from_entry)
            .ok_or_else(|| anyhow!("{NO_MODEL_HELP}")),
    }
}

/// Read the state from `--text`, else stdin. A terminal on stdin is refused
/// rather than waited on.
fn read_state(text: Option<&str>) -> Result<String> {
    let state = match text {
        Some(text) => text.to_string(),
        None => {
            if std::io::stdin().is_terminal() {
                bail!(
                    "pipe the text to decide on into this command (for example \
                     `biorouter decide --ask \"...\" < note.txt`), or pass --text or --batch"
                );
            }
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .context("could not read the text from stdin")?;
            buffer
        }
    };
    if state.trim().is_empty() {
        bail!("the text to decide on is empty");
    }
    Ok(state)
}

/// One file of a batch: its chunk, or the reason it is not one.
fn read_batch_file(path: &Path) -> std::result::Result<String, String> {
    let size = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("could not be read: {e}"))?;
    if size > MAX_BATCH_FILE_BYTES {
        return Err(format!(
            "is {} bytes, over the {} byte limit for one decision; split it",
            group_thousands(size as usize),
            group_thousands(MAX_BATCH_FILE_BYTES as usize)
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|_| "is not UTF-8 text".to_string())?;
    if text.trim().is_empty() {
        return Err("is empty".to_string());
    }
    Ok(text)
}

/// The regular, visible files of `dir`, sorted by name.
pub fn batch_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("could not read the directory {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.starts_with('.'))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("{} holds no files to decide on", dir.display());
    }
    Ok(files)
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `biorouter decide`.
pub async fn handle_decide(options: DecideOptions) -> Result<()> {
    let question = build_question(&options)?;
    let wanted = match options.model.as_deref() {
        Some(raw) => Some(DecisionModelRef::parse(raw).ok_or_else(|| {
            anyhow!("--model takes `provider/model`, for example `ollama_decision/tev1:0.8b`")
        })?),
        None => None,
    };
    let service = biorouter::decision::global();
    let (session, model) = chat_and_model(&options, service, wanted).await?;

    let target = Target {
        service,
        session: session.as_ref(),
        model: model.as_ref(),
        template: &DecisionRequest {
            questions: std::iter::once((options.name.clone(), question.clone())).collect(),
            context: DecisionContext::Text(String::new()),
        },
        question: &question,
    };
    match options.batch.as_deref() {
        None => decide_text(&options, &target).await,
        Some(dir) => decide_directory(&options, &target, dir).await,
    }
}

/// What every decision of one `biorouter decide` run is asked against.
struct Target<'a> {
    service: &'static DecisionService,
    session: Option<&'a DecisionSession>,
    model: Option<&'a DecisionModelRef>,
    /// The questions, held in a request so this module needs no direct
    /// dependency on the map crate that names their type.
    template: &'a DecisionRequest,
    question: &'a Question,
}

/// The chat, when `--session` names one (checked as `POST /decisions/decide`
/// checks it, and logged into), and the model: the one asked for, else for a
/// chat the service's own choice, else the ad hoc resolution, which warns
/// about a public model.
async fn chat_and_model(
    options: &DecideOptions,
    service: &'static DecisionService,
    wanted: Option<DecisionModelRef>,
) -> Result<(Option<DecisionSession>, Option<DecisionModelRef>)> {
    match options.session.as_deref() {
        Some(session_id) => {
            let manager = Arc::new(SessionManager::instance());
            let enforced = biorouter::privacy::privacy_tiers_enabled();
            // The reach rule `POST /decisions/decide` applies to a named chat,
            // so naming a chat here reaches no further than the route would.
            let capability = crate::session::privacy::caller_capability().await;
            super::session_decisions::check_local_reach(
                &manager,
                session_id,
                capability,
                enforced,
                SESSION_OUT_OF_REACH,
            )
            .await?;
            biorouter::decision::log::install_global_sink(manager.clone());
            let session = DecisionSession::load(manager, session_id, enforced)
                .await
                .map_err(|_| anyhow!("Session '{session_id}' not found."))?
                .with_origin(DecisionOrigin::Adhoc, None)
                .with_origin_label(Some("biorouter decide".to_string()));
            Ok((Some(session), wanted))
        }
        None => {
            let resolved = resolve_adhoc(service, wanted).await?;
            if resolved.tier == ProviderTier::Public {
                eprintln!("{}", public_model_warning(&resolved.display));
            }
            Ok((None, Some(resolved.model_ref)))
        }
    }
}

/// One state, from `--text` or stdin.
async fn decide_text(options: &DecideOptions, target: &Target<'_>) -> Result<()> {
    let state = read_state(options.text.as_deref())?;
    let request = DecisionRequest {
        questions: target.template.questions.clone(),
        context: DecisionContext::Text(state),
    };
    let decisions = target
        .service
        .decide(target.session, target.model, &request)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "decisions": decisions }))?
        );
    } else {
        for decision in &decisions {
            for line in render_decision(decision, Some(target.question)) {
                println!("{line}");
            }
        }
        eprintln!("{PROBABILITY_NOTE}");
    }
    Ok(())
}

/// Every file of `--batch`, one chunk each, in groups the service takes.
async fn decide_directory(options: &DecideOptions, target: &Target<'_>, dir: &Path) -> Result<()> {
    let files = batch_files(dir)?;
    let mut rows: Vec<Option<BatchRow>> = vec![None; files.len()];
    let mut chunks: Vec<(usize, DecisionChunk)> = Vec::new();
    for (index, path) in files.iter().enumerate() {
        let label = file_label(path);
        match read_batch_file(path) {
            Ok(text) => chunks.push((
                index,
                DecisionChunk {
                    id: label,
                    context: DecisionContext::Text(text),
                },
            )),
            Err(why) => {
                rows[index] = Some(BatchRow {
                    error: Some(format!("{label} {why}")),
                    file: label,
                    decision: None,
                })
            }
        }
    }
    for group in chunks.chunks(BATCH_GROUP) {
        let positions: Vec<usize> = group.iter().map(|(index, _)| *index).collect();
        let outcomes = target
            .service
            .decide_many(
                target.session,
                target.model,
                &target.template.questions,
                group.iter().map(|(_, chunk)| chunk.clone()).collect(),
            )
            .await
            .map_err(|e| anyhow!("{e}"))?;
        for (index, outcome) in positions.into_iter().zip(outcomes) {
            rows[index] = Some(BatchRow::from_outcome(outcome));
        }
    }
    let rows: Vec<BatchRow> = rows.into_iter().flatten().collect();
    if options.json {
        let summary = summarize_batch(&rows);
        let files: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "file": row.file,
                    "decision": row.decision,
                    "error": row.error,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "question": options.name,
                "files": files,
                "summary": summary,
            }))?
        );
    } else {
        for line in render_batch(&rows, target.question) {
            println!("{line}");
        }
        eprintln!("{PROBABILITY_NOTE}");
    }
    Ok(())
}

/// `biorouter models decision list`.
pub async fn handle_models_decision_list(format: &str) -> Result<()> {
    let entries = biorouter::decision::global().models().await;
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for line in render_model_list(&entries) {
            println!("{line}");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use biorouter::decision::types::{DecisionAdapter, DecisionKindTag, DecisionUsage};
    use clap::Parser;

    #[derive(Parser)]
    struct Harness {
        #[command(flatten)]
        options: DecideOptions,
    }

    fn parse(args: &[&str]) -> std::result::Result<DecideOptions, clap::Error> {
        let mut argv = vec!["decide"];
        argv.extend_from_slice(args);
        Harness::try_parse_from(argv).map(|h| h.options)
    }

    /// A decision built by hand, the way the service would build it.
    pub(crate) fn decision(
        question: &str,
        probabilities: &[(&str, f64)],
        answer: Answer,
    ) -> Decision {
        Decision {
            question: question.to_string(),
            answer,
            probabilities: probabilities
                .iter()
                .map(|(k, p)| (k.to_string(), *p))
                .collect(),
            confidence: 0.5,
            passed: None,
            pass_probability: None,
            source: ProbabilitySource::Native,
            calibrated: true,
            model: DecisionModelRef::new("ollama_decision", "tev1:0.8b"),
            tier: ProviderTier::Private,
            request_id: "r1".into(),
            latency_ms: 22,
            usage: DecisionUsage::default(),
            raw: serde_json::Value::Null,
        }
    }

    pub(crate) fn yes_no(p_yes: f64) -> Decision {
        decision(
            "has_ms",
            &[("yes", p_yes), ("no", 1.0 - p_yes)],
            Answer::Binary {
                value: p_yes >= 0.5,
            },
        )
    }

    #[test]
    fn the_options_parse_into_one_question_of_each_kind() {
        let binary = parse(&["--ask", "Is EDSS stated?", "--threshold", "0.8"]).unwrap();
        let question = build_question(&binary).unwrap();
        assert_eq!(question.kind, DecisionKind::Binary {});
        assert_eq!(question.threshold, Some(Threshold::Yes(0.8)));
        assert_eq!(binary.name, DEFAULT_QUESTION_NAME);

        let choice = parse(&[
            "--ask",
            "Which course?",
            "--choice",
            "rrms=Relapsing-remitting",
            "--choice",
            "spms=\"Secondary progressive\"",
            "--choice",
            "none",
            "--threshold",
            "0.7",
            "--threshold-on",
            "rrms",
        ])
        .unwrap();
        let question = build_question(&choice).unwrap();
        let DecisionKind::Choice { options } = &question.kind else {
            panic!("a choice question");
        };
        assert_eq!(
            options.iter().collect::<Vec<_>>(),
            vec![
                (
                    &"rrms".to_string(),
                    &Some("Relapsing-remitting".to_string())
                ),
                (
                    &"spms".to_string(),
                    &Some("Secondary progressive".to_string())
                ),
                (&"none".to_string(), &None),
            ],
            "option order is kept and the description is separate from the name"
        );
        assert_eq!(
            question.threshold,
            Some(Threshold::Option {
                name: "rrms".into(),
                p: 0.7
            })
        );

        let score = parse(&[
            "--ask",
            "How severe?",
            "--levels",
            "mild, moderate,severe",
            "--threshold",
            "0.6",
            "--threshold-on",
            "moderate",
        ])
        .unwrap();
        let question = build_question(&score).unwrap();
        assert_eq!(
            question.kind,
            DecisionKind::Score {
                levels: vec!["mild".into(), "moderate".into(), "severe".into()]
            }
        );
        assert_eq!(
            question.threshold,
            Some(Threshold::AtLeastLevel { level: 1, p: 0.6 })
        );
        let by_number = parse(&[
            "--ask",
            "How severe?",
            "--levels",
            "mild,severe",
            "--threshold",
            "0.6",
            "--threshold-on",
            "1",
        ])
        .unwrap();
        assert_eq!(
            build_question(&by_number).unwrap().threshold,
            Some(Threshold::AtLeastLevel { level: 1, p: 0.6 })
        );
    }

    #[test]
    fn mistakes_are_refused_before_anything_is_sent() {
        assert!(parse(&[]).is_err(), "--ask is required");
        assert!(parse(&["--ask", "q", "--threshold", "1.5"]).is_err());
        assert!(parse(&["--ask", "q", "--threshold-on", "yes"]).is_err());
        assert!(parse(&["--ask", "q", "--choice", "a", "--levels", "x,y"]).is_err());
        assert!(parse(&["--ask", "q", "--text", "t", "--batch", "/tmp"]).is_err());
        assert!(parse(&["--ask", "q", "--choice", "=desc"]).is_err());

        let one_option = parse(&["--ask", "q", "--choice", "only"]).unwrap();
        assert!(build_question(&one_option)
            .unwrap_err()
            .to_string()
            .contains("2 to 26 options"));
        let no_on = parse(&[
            "--ask",
            "q",
            "--choice",
            "a",
            "--choice",
            "b",
            "--threshold",
            "0.5",
        ])
        .unwrap();
        assert!(build_question(&no_on)
            .unwrap_err()
            .to_string()
            .contains("--threshold-on"));
        let wrong_option = parse(&[
            "--ask",
            "q",
            "--choice",
            "a",
            "--choice",
            "b",
            "--threshold",
            "0.5",
            "--threshold-on",
            "c",
        ])
        .unwrap();
        assert!(build_question(&wrong_option)
            .unwrap_err()
            .to_string()
            .contains("not one of its options"));
        let bad_yes = parse(&[
            "--ask",
            "q",
            "--threshold",
            "0.5",
            "--threshold-on",
            "maybe",
        ])
        .unwrap();
        assert!(build_question(&bad_yes).is_err());
        let bad_name = parse(&["--ask", "q", "--name", "has ms"]).unwrap();
        assert!(build_question(&bad_name).is_err());
        let eleven = parse(&["--ask", "q", "--levels", "a,b,c,d,e,f,g,h,i,j,k"]).unwrap();
        assert!(build_question(&eleven).is_err());
    }

    #[test]
    fn a_bar_is_filled_in_eighths_on_a_fixed_width_track() {
        assert_eq!(probability_bar(0.0, 4), "░░░░");
        assert_eq!(probability_bar(1.0, 4), "████");
        assert_eq!(probability_bar(0.5, 4), "██░░");
        assert_eq!(
            probability_bar(0.5625, 4),
            "██▎░",
            "2.25 cells: two full and two eighths"
        );
        assert_eq!(probability_bar(f64::NAN, 3), "░░░");
        assert_eq!(probability_bar(2.0, 3), "███", "clamped");
        for p in [0.0, 0.13, 0.5, 0.77, 0.999, 1.0] {
            assert_eq!(probability_bar(p, BAR_WIDTH).chars().count(), BAR_WIDTH);
        }
    }

    #[test]
    fn one_decision_reads_as_bars_then_the_answer_line() {
        let question = Question {
            instructions: "Does this note state the patient has MS?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.8)),
        };
        let mut d = yes_no(0.82);
        d.passed = Some(true);
        d.pass_probability = Some(0.82);
        let lines = render_decision(&d, Some(&question));
        assert_eq!(lines[0], "Does this note state the patient has MS?");
        assert!(lines[1].starts_with("▸ yes  "), "{}", lines[1]);
        assert!(lines[1].ends_with("0.82"), "{}", lines[1]);
        assert!(lines[2].starts_with("  no   "), "{}", lines[2]);
        assert_eq!(
            lines[3],
            "Answer: yes  p 0.82  calibrated  ollama_decision/tev1:0.8b  22 ms"
        );
        assert_eq!(lines[4], "Threshold: P(yes) 0.82 ≥ 0.80, passed");

        let mut low = yes_no(0.31);
        low.passed = Some(false);
        low.pass_probability = Some(0.31);
        low.calibrated = false;
        low.source = ProbabilitySource::Verbalized { reason: None };
        let lines = render_decision(&low, Some(&question));
        assert!(
            lines[2].starts_with("▸ no"),
            "the answer is marked: {lines:?}"
        );
        assert!(lines[3].contains("Answer: no  p 0.69  uncalibrated (the model wrote it)"));
        assert_eq!(lines[4], "Threshold: P(yes) 0.31 < 0.80, failed");
    }

    #[test]
    fn a_score_is_drawn_by_its_levels_and_gated_from_a_level_up() {
        let question = Question {
            instructions: "How severe?".into(),
            kind: DecisionKind::Score {
                levels: vec!["mild".into(), "moderate".into(), "severe".into()],
            },
            threshold: Some(Threshold::AtLeastLevel { level: 1, p: 0.5 }),
        };
        let mut d = decision(
            "severity",
            &[("0", 0.2), ("1", 0.5), ("2", 0.3)],
            Answer::Score {
                level: 1,
                label: "moderate".into(),
                expected: 1.1,
            },
        );
        d.passed = Some(true);
        d.pass_probability = Some(0.8);
        d.latency_ms = 1_250;
        let lines = render_decision(&d, Some(&question));
        assert!(lines[1].starts_with("  mild    "), "{lines:?}");
        assert!(lines[2].starts_with("▸ moderate"), "{lines:?}");
        assert!(lines[4].contains("Answer: moderate  p 0.50"), "{lines:?}");
        assert!(lines[4].ends_with("1.2 s"), "{lines:?}");
        assert_eq!(
            lines[5],
            "Threshold: P(at least moderate) 0.80 ≥ 0.50, passed"
        );
    }

    #[test]
    fn a_batch_prints_one_row_per_file_then_a_summary_and_the_errors() {
        let question = Question {
            instructions: "Does this note state MS?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.9)),
        };
        let pass = |p: f64| {
            let mut d = yes_no(p);
            d.pass_probability = Some(p);
            d.passed = Some(p >= 0.9);
            d
        };
        let mut uncalibrated = pass(0.2);
        uncalibrated.calibrated = false;
        let rows = vec![
            BatchRow {
                file: "note-0001.txt".into(),
                decision: Some(pass(0.97)),
                error: None,
            },
            BatchRow {
                file: "note-0002.txt".into(),
                decision: Some(uncalibrated),
                error: None,
            },
            BatchRow {
                file: "note-0003.txt".into(),
                decision: Some(pass(0.95)),
                error: None,
            },
            BatchRow {
                file: "note-0004.txt".into(),
                decision: None,
                error: Some("Chunk `note-0004.txt` is about 20000 tokens".into()),
            },
        ];
        let summary = summarize_batch(&rows);
        assert_eq!(
            summary,
            BatchSummary {
                files: 4,
                decided: 3,
                errors: 1,
                answers: vec![("yes".into(), 2), ("no".into(), 1)],
                passed: Some(2),
                uncalibrated: 1,
            }
        );
        assert_eq!(
            batch_summary_line(&summary),
            "Summary: 4 files · yes 2 · no 1 · 2 passed the threshold · 1 error · 1 uncalibrated"
        );
        let lines = render_batch(&rows, &question);
        assert_eq!(lines[0], "Does this note state MS?");
        assert!(lines[1].contains("file") && lines[1].contains("outcome"));
        assert!(lines[2].contains("note-0001.txt") && lines[2].contains("yes"));
        assert!(lines[2].contains("0.97") && lines[2].ends_with("passed"));
        assert!(lines[3].contains("no") && lines[3].ends_with("failed"));
        assert!(lines[5].contains("note-0004.txt") && lines[5].ends_with("error"));
        assert!(lines[6].starts_with("Summary: 4 files"));
        assert_eq!(lines[7], "Errors:");
        assert!(lines[8].contains("about 20000 tokens"));
    }

    #[test]
    fn a_batch_reads_the_visible_files_of_a_directory_in_name_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "second").unwrap();
        std::fs::write(dir.path().join("a.txt"), "first").unwrap();
        std::fs::write(dir.path().join(".hidden"), "skip").unwrap();
        std::fs::write(dir.path().join("empty.txt"), "  \n").unwrap();
        std::fs::write(dir.path().join("binary.bin"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let files = batch_files(dir.path()).unwrap();
        let names: Vec<String> = files.iter().map(|p| file_label(p)).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "binary.bin", "empty.txt"]);
        assert_eq!(read_batch_file(&files[0]).unwrap(), "first");
        assert_eq!(read_batch_file(&files[2]).unwrap_err(), "is not UTF-8 text");
        assert_eq!(read_batch_file(&files[3]).unwrap_err(), "is empty");

        let empty = tempfile::tempdir().unwrap();
        assert!(batch_files(empty.path()).is_err());
    }

    #[test]
    fn the_model_list_names_tier_context_kinds_and_adapter() {
        assert_eq!(render_model_list(&[]), vec![NO_MODEL_HELP.to_string()]);
        let entries = vec![
            DecisionModelEntry {
                provider: "ollama_decision".into(),
                model: "tev1:0.8b".into(),
                display_name: "tev1:0.8b".into(),
                context_tokens: 8_192,
                kinds: vec![
                    DecisionKindTag::Binary,
                    DecisionKindTag::Choice,
                    DecisionKindTag::Score,
                ],
                tier: ProviderTier::Private,
                affiliation: None,
                adapter: DecisionAdapter::SystemOne,
                local: true,
            },
            DecisionModelEntry {
                provider: "baseten_decision".into(),
                model: "decision-models/elephant-1".into(),
                display_name: "elephant-1".into(),
                context_tokens: 16_384,
                kinds: vec![DecisionKindTag::Binary],
                tier: ProviderTier::Public,
                affiliation: None,
                adapter: DecisionAdapter::SystemOne,
                local: false,
            },
        ];
        let lines = render_model_list(&entries);
        assert_eq!(lines[0], "Decision models (2, local first):");
        assert!(lines[1].contains("ollama_decision/tev1:0.8b"));
        assert!(lines[1].contains(
            "private · this computer · 8,192 tokens · yes/no, choice, score · system_one"
        ));
        assert!(lines[2].contains("public · 16,384 tokens · yes/no · system_one"));
    }

    #[test]
    fn the_public_warning_is_one_line_naming_the_model() {
        let line = public_model_warning("elephant-1");
        assert!(!line.contains('\n'));
        assert!(line.starts_with("warning: elephant-1 is hosted outside your institution"));
    }

    #[test]
    fn small_formatters() {
        assert_eq!(format_latency(22), "22 ms");
        assert_eq!(format_latency(1_000), "1.0 s");
        assert_eq!(
            short_model(&DecisionModelRef::new(
                "baseten_decision",
                "decision-models/joey-1"
            )),
            "joey-1"
        );
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        assert_eq!(group_thousands(12), "12");
        assert_eq!(clip("a  b\nc", 10), "a b c");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        assert_eq!(
            parse_choice("rrms = 'Relapsing'").unwrap(),
            ("rrms".to_string(), Some("Relapsing".to_string()))
        );
        assert_eq!(parse_choice("none=").unwrap(), ("none".to_string(), None));
        let mut logprob = yes_no(0.6);
        logprob.calibrated = false;
        logprob.source = ProbabilitySource::Logprob { mass: 0.7 };
        assert_eq!(
            calibration_flag(&logprob),
            "uncalibrated (logprob, 70% on the options)"
        );
    }
}
