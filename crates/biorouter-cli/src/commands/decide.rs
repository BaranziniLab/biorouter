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
//! The model is chosen once, by the service's own order (the one named, the
//! chat's default, the configured default, the automatic choice), and pinned
//! for every request of the run, so the warning below names the model that
//! reads the text and a long `--batch` never moves to another model partway.
//!
//! * **With `--session <id>`** the decision is about that chat: the service
//!   reads the chat's classification and refuses a model outside its tier
//!   before anything is sent, and the call is written to the chat's decision
//!   log with the `adhoc` origin. The text is the caller's, not the chat's,
//!   so when the chat is not private stderr says the text is kept where
//!   public chats can read it.
//! * **Without one** nothing is logged and no tier is enforced, because there is
//!   no chat to protect. The text is the person's own. The provider still
//!   bills the call and no usage record holds it, so the run ends with one
//!   stderr line giving the tokens used, and the dollars when the model has a
//!   price. With `--session` the usage is in the chat's usage ledger instead.
//!
//! Either way, when a model hosted outside the institution will read the
//! text, one line on stderr says so before anything is sent. The person is
//! warned; the agent never is the one choosing here.
//!
//! The renderers below are pure functions over decision values, shared with
//! `session decisions` and the TUI, so every terminal surface draws a
//! probability the same way.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use biorouter::decision::types::validate_questions;
use biorouter::decision::{
    Answer, ChunkOutcome, Decision, DecisionChunk, DecisionContext, DecisionError, DecisionKind,
    DecisionModelEntry, DecisionModelRef, DecisionOrigin, DecisionRequest, DecisionResult,
    DecisionService, DecisionSession, DecisionUsage, ModelSource, ProbabilitySource, Question,
    Threshold,
};
use biorouter::privacy::ProviderTier;
use biorouter::providers::decision::catalog::{
    baseten_display_name, ollama_base_url, BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER,
};
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
/// How long a run about a chat waits, before it exits, for decision requests
/// that answered after its deadline or after it stopped waiting, so their
/// usage is stored and counted.
pub const LATE_ANSWER_GRACE: Duration = Duration::from_secs(30);
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

/// What an empty `--ask` is told, here and by `session monitors add`.
pub const EMPTY_ASK: &str =
    "--ask needs a question, for example --ask \"Is an EDSS score stated?\"";

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

/// A monitor's `--threshold`: a probability above 0, at most 1. A monitor at 0
/// would fire on every check, so the store refuses it; refusing it here gives
/// 0 and 1.2 the same sentence, the one every probability flag of the
/// command uses.
pub fn parse_positive_probability(raw: &str) -> Result<f64, String> {
    let p = parse_probability(raw).map_err(|_| positive_probability_message(raw))?;
    if p <= 0.0 {
        return Err(positive_probability_message(raw));
    }
    Ok(p)
}

fn positive_probability_message(raw: &str) -> String {
    format!("{raw} is not a probability above 0 and at most 1")
}

/// The one question the options describe, checked the way the service checks
/// it, so a mistake is reported before anything is sent.
pub fn build_question(options: &DecideOptions) -> Result<Question> {
    if options.ask.trim().is_empty() {
        bail!("{EMPTY_ASK}");
    }
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

/// How a terminal names a decision model: a known Baseten slug by its alias
/// (`decision-models/baseten-joey-2b-latest` is `joey-1`), an unknown one
/// without its `decision-models/` prefix, any other model by its id. Records,
/// the decision log and the wire keep the slug; only what is printed changes.
pub fn model_name(provider: &str, model: &str) -> String {
    if provider == BASETEN_DECISION_PROVIDER {
        baseten_display_name(model)
    } else {
        model.to_string()
    }
}

/// `provider/name` by [`model_name`]: `baseten_decision/joey-1`, the name
/// `--model` takes and the run's usage line prints.
pub fn model_label(model: &DecisionModelRef) -> String {
    terminal_text(&format!(
        "{}/{}",
        model.provider,
        model_name(&model.provider, &model.model)
    ))
}

/// The short name: a Baseten decision model's alias (`joey-1`), else the last
/// segment of the model id (`tev1:0.8b`).
pub fn short_model(model: &DecisionModelRef) -> String {
    if model.provider == BASETEN_DECISION_PROVIDER {
        return terminal_text(&baseten_display_name(&model.model));
    }
    terminal_text(model.model.rsplit('/').next().unwrap_or(&model.model))
}

/// Where a decision model runs, as a person reads it beside the model's name:
/// `this computer` for an Ollama model on a loopback address, `Ollama cloud`
/// for an Ollama cloud tag (`gpt-oss:120b-cloud`) whatever the host, `Ollama`
/// for one on another host, `Baseten` for Baseten's. `None` for any other
/// provider. The place is the model's, not only the provider's: a loopback
/// Ollama forwards a cloud tag to ollama.com, so naming it `this computer`
/// would say text stays here when it does not (the catalog lists such a model
/// as remote and Public).
pub fn model_place(provider: &str, model: &str) -> Option<&'static str> {
    match provider {
        BASETEN_DECISION_PROVIDER => Some("Baseten"),
        OLLAMA_DECISION_PROVIDER => Some(ollama_place(model, ollama_is_this_computer())),
        _ => None,
    }
}

/// [`model_place`] for an Ollama model, with whether `OLLAMA_HOST` names this
/// computer given. ⚠ A cloud model renamed with `ollama cp` carries no cloud
/// tag, and only `/api/show` tells; a line that names a model from its id
/// alone cannot ask, so the list, which reads each entry's `local` flag from
/// the catalog, is the authority there.
pub(crate) fn ollama_place(model: &str, host_is_this_computer: bool) -> &'static str {
    if biorouter::providers::utils::is_ollama_cloud_model(model) {
        "Ollama cloud"
    } else if host_is_this_computer {
        "this computer"
    } else {
        "Ollama"
    }
}

/// Whether `OLLAMA_HOST` (or its default) names this computer, read once.
fn ollama_is_this_computer() -> bool {
    static LOCAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOCAL.get_or_init(|| {
        ollama_base_url()
            .ok()
            .is_some_and(|url| host_is_loopback(url.host_str().unwrap_or_default()))
    })
}

/// `localhost`, `127.0.0.1` and the rest of 127/8, `::1`.
pub fn host_is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// A decision model as a person reads it where it is not a value they type
/// back: its short name and where it runs, `joey-1 · Baseten`,
/// `tev1:0.8b · this computer`. `--model` and the lines that show what to type
/// keep `provider/model` ([`model_label`]).
pub fn model_for_person(model: &DecisionModelRef) -> String {
    let name = short_model(model);
    match model_place(&model.provider, &model.model) {
        Some(place) => format!("{name} · {place}"),
        None => name,
    }
}

/// Every `provider/model` a supervision line names, as [`model_for_person`]
/// names it: `baseten_decision/decision-models/baseten-joey-2b-latest` is
/// `joey-1 · Baseten`. The lines are built where the agent's tools and the
/// desktop share them; the terminal names the models as it names them
/// everywhere else. A name with no slash or an unknown provider is left.
pub fn person_model_names(text: &str) -> String {
    replace_model_ids(text, model_for_person)
}

/// [`person_model_names`] for a sentence, where the place reads better in
/// parentheses: "the chat's default decision model joey-1 (Baseten) could
/// not run".
pub fn person_model_names_in_sentence(text: &str) -> String {
    replace_model_ids(text, |model| {
        match model_place(&model.provider, &model.model) {
            Some(place) => format!("{} ({place})", short_model(model)),
            None => short_model(model),
        }
    })
}

fn replace_model_ids(text: &str, name: impl Fn(&DecisionModelRef) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next = [BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER]
            .iter()
            .filter_map(|provider| {
                let prefix = format!("{provider}/");
                rest.split_once(&prefix)
                    .map(|(before, after)| (before, *provider, prefix, after))
            })
            .min_by_key(|(before, ..)| before.len());
        let Some((before, provider, prefix, after)) = next else {
            break;
        };
        out.push_str(before);
        // A model id runs to the next space or quote; a sentence's last
        // full stop or comma is not part of it.
        let mut model = after
            .split_once(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ';' | ')'))
            .map_or(after, |(id, _)| id);
        while let Some(stripped) = model.strip_suffix(['.', ',', ':']) {
            model = stripped;
        }
        if model.is_empty() {
            out.push_str(&prefix);
            rest = after;
            continue;
        }
        out.push_str(&name(&DecisionModelRef::new(provider, model)));
        // `model` is where `after` starts, so this always strips it.
        rest = after.strip_prefix(model).unwrap_or(after);
    }
    out.push_str(rest);
    out
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

/// What a threshold compares and against what: `P(yes)` and 0.80. A `below`
/// threshold compares what its inner threshold does.
pub fn threshold_terms(threshold: &Threshold, question: &Question) -> (String, f64) {
    match threshold {
        Threshold::Yes(t) => ("P(yes)".to_string(), *t),
        Threshold::No(t) => ("P(no)".to_string(), *t),
        Threshold::Option { name, p } => (format!("P({name})"), *p),
        Threshold::AtLeastLevel { level, p } => {
            let label = match &question.kind {
                DecisionKind::Score { levels } => levels.get(*level).cloned(),
                _ => None,
            }
            .unwrap_or_else(|| format!("level {level}"));
            (format!("P(at least {label})"), *p)
        }
        Threshold::Below(inner) => threshold_terms(inner, question),
    }
}

/// What the threshold compared, as a sentence: `P(yes) 0.82 ≥ 0.80, passed`,
/// or `P(yes) 0.40 < 0.80, not passed` (never "failed": a person reads that
/// as an error). A `below` threshold passes under its bound:
/// `P(yes) 0.12 < 0.30, passed`.
pub fn threshold_line(decision: &Decision, question: Option<&Question>) -> Option<String> {
    let question = question?;
    let threshold = question.threshold.as_ref()?;
    let p = decision.pass_probability?;
    let (wanted, bound) = threshold_terms(threshold, question);
    let passed = decision.passed == Some(true);
    let sign = if passed ^ threshold.is_below() {
        "≥"
    } else {
        "<"
    };
    Some(format!(
        "{wanted} {} {sign} {}, {}",
        format_p(p),
        format_p(bound),
        if passed { "passed" } else { "not passed" }
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
        model_label(&decision.model),
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
    /// What a failed file's request was billed for, when its model answered
    /// before it failed (an answer that could not be read).
    pub billed_usage: Option<DecisionUsage>,
}

impl BatchRow {
    fn from_outcome(outcome: ChunkOutcome) -> Self {
        Self {
            file: outcome.chunk_id,
            decision: outcome
                .decisions
                .and_then(|mut d| (!d.is_empty()).then(|| d.remove(0))),
            error: outcome.error,
            billed_usage: outcome.billed_usage,
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
                    Some(false) => "not passed",
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

/// The advice an agent is given in a decision-model refusal, as a person at
/// a terminal is given it: the agent lists the models with its
/// `decision_models` tool, a person with `biorouter models decision list`.
/// The refusal is built where the agent's and the person's share it, so the
/// terminal swaps the one phrase as it prints it.
pub fn terminal_advice(text: &str) -> String {
    text.replace(
        "(decision_models lists them)",
        "(`biorouter models decision list` shows them)",
    )
}

/// `error` with [`terminal_advice`] applied when its sentence carries the
/// agent's advice, else `error` itself, so a typed error (one that picks an
/// exit code) keeps its type.
pub fn advise(error: anyhow::Error) -> anyhow::Error {
    let text = error.to_string();
    let advised = terminal_advice(&text);
    if advised == text {
        error
    } else {
        anyhow!("{advised}")
    }
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
    // A Baseten model by its alias, as `--model` takes it; `--format json`
    // keeps the slug.
    let names: Vec<String> = entries
        .iter()
        .map(|e| {
            terminal_text(&format!(
                "{}/{}",
                e.provider,
                model_name(&e.provider, &e.model)
            ))
        })
        .collect();
    let name_w = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let mut out = vec![format!("Decision models ({}, local first):", entries.len())];
    for (entry, name) in entries.iter().zip(&names) {
        let kinds: Vec<&str> = entry
            .kinds
            .iter()
            .map(|kind| match kind {
                biorouter::decision::types::DecisionKindTag::Binary => "yes/no",
                biorouter::decision::types::DecisionKindTag::Choice => "choice",
                biorouter::decision::types::DecisionKindTag::Score => "score",
            })
            .collect();
        // The catalog's `local` flag is the authority on "this computer": it
        // reads `/api/show`, so a cloud model renamed with `ollama cp` is not
        // local either. Anything else from Ollama is named by its own place,
        // never this computer.
        let place = if entry.local {
            format!("{} · this computer", tier_word(entry.tier))
        } else if entry.provider == BASETEN_DECISION_PROVIDER {
            format!("{} · Baseten", tier_word(entry.tier))
        } else if entry.provider == OLLAMA_DECISION_PROVIDER {
            format!(
                "{} · {}",
                tier_word(entry.tier),
                ollama_place(&entry.model, false)
            )
        } else {
            tier_word(entry.tier).to_string()
        };
        let adapter = serde_json::to_value(entry.adapter)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let unloadable = if entry.load_refusal.is_some() {
            " · cannot be loaded"
        } else {
            ""
        };
        out.push(format!(
            "  {:<name_w$}  {place} · {} tokens · {} · {adapter}{unloadable}",
            name,
            group_thousands(entry.context_tokens),
            kinds.join(", ")
        ));
        if let Some(refusal) = &entry.load_refusal {
            out.push(format!("    {}", terminal_text(refusal)));
        }
    }
    out
}

/// What is said when no decision model can be used. It quotes
/// `biorouter::decision::INSTALL_ADVICE` word for word (a test holds the two
/// together; `concat!` cannot read a constant from another crate).
pub const NO_MODEL_HELP: &str =
    "No decision model is available. Install one on this computer with `biorouter models \
     decision pull` (tev1:0.8b, about 1.5 GB), or with Install in a chat's Monitors dialog. Or \
     add a Baseten API key, or set BIOROUTER_DECISION_MODEL.";

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

/// The model a run of `biorouter decide` uses, chosen once before anything is
/// sent so the person is warned about the model that will actually read the
/// text, and then pinned for every request of the run.
#[derive(Debug, Clone)]
struct ResolvedModel {
    model_ref: DecisionModelRef,
    display: String,
    tier: ProviderTier,
}

/// The model the service would use for `session` naming `wanted`, by the
/// service's own order (named, the chat's default, the configured default,
/// the automatic choice), so the terminal and the daemon never choose
/// differently. Read from the listing; a model the listing does not show is
/// opened to learn its tier, which sends its name and nothing else.
async fn resolve_run_model(
    service: &DecisionService,
    session: Option<&DecisionSession>,
    wanted: Option<&DecisionModelRef>,
) -> DecisionResult<ResolvedModel> {
    if let Some(entry) = service.model_entry_for(session, wanted).await {
        return Ok(ResolvedModel {
            model_ref: entry.model_ref(),
            display: entry.display_name.clone(),
            tier: entry.tier,
        });
    }
    let opened = service.resolve_for(session, wanted).await?;
    Ok(ResolvedModel {
        model_ref: opened.model_ref(),
        display: opened.display_name(),
        tier: opened.tier(),
    })
}

/// What stderr says before an ad hoc decision about a chat is sent. A public
/// model is named, as it is with no chat, unless the chat's tier refuses it
/// anyway (a private chat with privacy on). A chat that is not private keeps
/// the text in its decision history, at the chat's reach, so that is said too.
pub fn session_warnings(
    chat_private: bool,
    enforced: bool,
    display: &str,
    tier: ProviderTier,
) -> Vec<String> {
    let mut lines = Vec::new();
    if tier == ProviderTier::Public && !(chat_private && enforced) {
        lines.push(public_model_warning(display));
    }
    if enforced && !chat_private {
        lines.push(PUBLIC_CHAT_LOG_NOTE.to_string());
    }
    lines
}

/// What a terminal is told when it records a decision in a chat that is not
/// private.
pub const PUBLIC_CHAT_LOG_NOTE: &str =
    "note: this chat is not private, so the text you send is kept in its decision history, \
     where public chats can read it. For private text, leave out --session or name a private \
     chat.";

/// The requests `decisions` came from, and their usage summed once per
/// request: decisions that share a request id share its usage.
pub fn request_usage<'a>(
    decisions: impl IntoIterator<Item = &'a Decision>,
) -> (usize, DecisionUsage) {
    let mut seen = std::collections::HashSet::new();
    decisions
        .into_iter()
        .filter(|decision| seen.insert(decision.request_id.as_str()))
        .fold(
            (0, DecisionUsage::default()),
            |(requests, usage), decision| (requests + 1, usage + decision.usage),
        )
}

/// The requests a run's rows came from and their usage: every decided
/// request once ([`request_usage`]), and every failed one whose model
/// answered before it failed, which was billed all the same.
pub fn run_usage(rows: &[BatchRow]) -> (usize, DecisionUsage) {
    let decided = request_usage(rows.iter().filter_map(|row| row.decision.as_ref()));
    rows.iter()
        .filter(|row| row.decision.is_none())
        .filter_map(|row| row.billed_usage)
        .fold(decided, |(requests, usage), billed| {
            (requests + 1, usage + billed)
        })
}

/// What a run about a chat says on stderr while it waits for `pending`
/// decision requests to answer before it exits.
pub fn late_wait_line(pending: usize) -> String {
    format!(
        "waiting up to {} s for {pending} decision request(s) that answered late, so their \
         usage is counted",
        LATE_ANSWER_GRACE.as_secs()
    )
}

/// What a run about a chat says when `remaining` decision requests had not
/// answered when it gave up waiting.
pub fn late_unsettled_line(remaining: usize) -> String {
    format!(
        "note: {remaining} decision request(s) had not answered when this run ended; if the \
         provider bills them, Biorouter does not count it."
    )
}

/// Before a run about a chat exits: wait (up to [`LATE_ANSWER_GRACE`]) for
/// requests that answered after their deadline, so the chat's decision log
/// and usage count them, and say so.
async fn wait_for_late_answers() {
    let pending = biorouter::decision::late::in_flight();
    if pending == 0 {
        return;
    }
    eprintln!("{}", late_wait_line(pending));
    let remaining = biorouter::decision::settle_late_answers(LATE_ANSWER_GRACE).await;
    if remaining > 0 {
        eprintln!("{}", late_unsettled_line(remaining));
    }
}

/// `about $0.12`, or `less than $0.01` for the fractions of a cent most
/// decision runs cost.
fn format_usd(usd: f64) -> String {
    if usd < 0.005 {
        "less than $0.01".to_string()
    } else {
        format!("about ${usd:.2}")
    }
}

/// The stderr line a run with no `--session` ends with: the provider bills
/// the call and nothing in Biorouter records it, so the person is told what
/// it used. `model` is how the run names its model, `provider/name`
/// ([`PinnedModel::label`]). `cost` is the dollars when the model has a price,
/// `None` when it has none (a local Ollama model). `None` when the model
/// reported no usage.
pub fn usage_line(
    model: &str,
    requests: usize,
    usage: DecisionUsage,
    cost: Option<f64>,
) -> Option<String> {
    usage.billed_total()?;
    let tokens = |count: Option<i64>| {
        count.map_or_else(
            || "unknown".to_string(),
            |n| group_thousands(usize::try_from(n).unwrap_or(0)),
        )
    };
    let price = cost
        .filter(|usd| usd.is_finite() && *usd >= 0.0)
        .map_or_else(String::new, |usd| format!(", {}", format_usd(usd)));
    Some(format!(
        "usage: {} on {model}: {} input and {} output tokens{price}. Without --session this is \
         not recorded in Biorouter's usage.",
        plural(requests, "request", "requests"),
        tokens(usage.input_tokens),
        tokens(usage.output_tokens),
    ))
}

/// Print [`usage_line`] for a run that named no chat, after its results:
/// `requests` and their `usage` ([`request_usage`], [`run_usage`]).
fn report_unrecorded_usage(target: &Target<'_>, (requests, usage): (usize, DecisionUsage)) {
    if target.session.is_some() {
        return;
    }
    let RunModel::Pinned(pinned) = target.model else {
        return;
    };
    let model = &pinned.model_ref;
    let tokens = |count: Option<i64>| u64::try_from(count.unwrap_or(0)).unwrap_or(0);
    let cost = biorouter::providers::pricing::estimate_cost_usd(
        &model.provider,
        &model.model,
        tokens(usage.input_tokens),
        tokens(usage.output_tokens),
    );
    if let Some(line) = usage_line(&pinned.label, requests, usage, cost) {
        eprintln!("{line}");
    }
}

/// The model a run is pinned to, and how the run names it.
#[derive(Debug, Clone, PartialEq)]
struct PinnedModel {
    model_ref: DecisionModelRef,
    /// `provider/name`, by the listing's name (`baseten_decision/mammoth-1`),
    /// which is what `--model` takes and the docs show, not the Baseten slug.
    label: String,
    /// Where the model came from (`--model` or the chat's own default, else
    /// the configured default or the automatic choice), by the service's own
    /// rule, so a privacy refusal advises what would change it. Every request
    /// names the pinned model, which the service would otherwise read as
    /// `--model`.
    source: ModelSource,
}

/// The model a run sends to, or, for a run about a chat, why none could be
/// chosen. That run then logs the failure in the chat's decision history once
/// it has read the text, as `POST /decisions/decide` does, and fails. A run
/// with no chat has no history, so it fails at once.
#[derive(Debug)]
enum RunModel {
    Pinned(PinnedModel),
    Unresolved(DecisionError),
}

/// Resolve the run's model once and say what must be said before sending.
async fn pin_model(
    service: &DecisionService,
    session: Option<&DecisionSession>,
    wanted: Option<DecisionModelRef>,
) -> DecisionResult<(PinnedModel, Vec<String>)> {
    let source = ModelSource::of(session, wanted.as_ref());
    let resolved = resolve_run_model(service, session, wanted.as_ref()).await?;
    let lines = match session {
        Some(session) => session_warnings(
            session.classification.is_private(),
            session.enforced,
            &resolved.display,
            resolved.tier,
        ),
        None if resolved.tier == ProviderTier::Public => {
            vec![public_model_warning(&resolved.display)]
        }
        None => Vec::new(),
    };
    let label = format!("{}/{}", resolved.model_ref.provider, resolved.display);
    Ok((
        PinnedModel {
            model_ref: resolved.model_ref,
            label,
            source,
        },
        lines,
    ))
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

/// What a run says when `BIOROUTER_DECISION_MODEL` names a model with no
/// provider (`tev1:0.8b` rather than `ollama_decision/tev1:0.8b`) and no
/// `BIOROUTER_DECISION_PROVIDER` is set: the service cannot tell which
/// provider serves it, so it is not used, and the run would otherwise choose
/// another model without a word. `provider` and `model` are the two values,
/// trimmed, as the configuration holds them.
pub fn ignored_default_model_warning(
    provider: Option<&str>,
    model: Option<&str>,
) -> Option<String> {
    let model = model.map(str::trim).filter(|m| !m.is_empty())?;
    if provider.map(str::trim).is_some_and(|p| !p.is_empty()) || model.contains('/') {
        return None;
    }
    Some(format!(
        "warning: BIOROUTER_DECISION_MODEL is {model}, which names no provider, so it is not \
         used and the model is chosen automatically. Write it provider/model (for example \
         ollama_decision/tev1:0.8b), or set BIOROUTER_DECISION_PROVIDER as well."
    ))
}

/// [`ignored_default_model_warning`] for this process's configuration.
fn configured_default_warning() -> Option<String> {
    let config = biorouter::config::Config::global();
    let read = |key: &str| config.get_param::<String>(key).ok();
    ignored_default_model_warning(
        read("BIOROUTER_DECISION_PROVIDER").as_deref(),
        read("BIOROUTER_DECISION_MODEL").as_deref(),
    )
}

/// `biorouter decide`.
pub async fn handle_decide(options: DecideOptions) -> Result<()> {
    let question = build_question(&options)?;
    if options.model.is_none() {
        if let Some(warning) = configured_default_warning() {
            eprintln!("{warning}");
        }
    }
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
        model: &model,
        template: &DecisionRequest {
            questions: std::iter::once((options.name.clone(), question.clone())).collect(),
            context: DecisionContext::Text(String::new()),
        },
        question: &question,
    };
    let result = match options.batch.as_deref() {
        None => decide_text(&options, &target).await,
        Some(dir) => decide_directory(&options, &target, dir).await,
    };
    // A late answer is stored and billed only for a chat, so only a run
    // about one waits for them.
    if target.session.is_some() {
        wait_for_late_answers().await;
    }
    result
}

/// What every decision of one `biorouter decide` run is asked against.
struct Target<'a> {
    service: &'static DecisionService,
    session: Option<&'a DecisionSession>,
    model: &'a RunModel,
    /// The questions, held in a request so this module needs no direct
    /// dependency on the map crate that names their type.
    template: &'a DecisionRequest,
    question: &'a Question,
}

/// The chat, when `--session` names one (checked as `POST /decisions/decide`
/// checks it, and logged into), and the model, resolved once by the service's
/// own order and pinned for the whole run, so a long `--batch` never moves to
/// another model partway (an Ollama that stops answering lists nothing, and
/// the automatic choice would then pick a hosted model). If the pinned model
/// goes away, the run fails rather than sending the rest elsewhere. When no
/// model can be chosen for the chat, the failure is returned as
/// [`RunModel::Unresolved`], so the run logs it in the chat before it fails.
async fn chat_and_model(
    options: &DecideOptions,
    service: &'static DecisionService,
    wanted: Option<DecisionModelRef>,
) -> Result<(Option<DecisionSession>, RunModel)> {
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
            let model = run_model_for_chat(service, &session, wanted).await;
            Ok((Some(session), model))
        }
        None => {
            let (model, lines) = pin_model(service, None, wanted)
                .await
                .map_err(|e| anyhow!("{}", terminal_advice(&e.to_string())))?;
            for line in lines {
                eprintln!("{line}");
            }
            Ok((None, RunModel::Pinned(model)))
        }
    }
}

/// The run model for a run about `session`, its warnings printed. A failure
/// is kept for the run to log in the chat (see [`RunModel`]).
async fn run_model_for_chat(
    service: &DecisionService,
    session: &DecisionSession,
    wanted: Option<DecisionModelRef>,
) -> RunModel {
    match pin_model(service, Some(session), wanted).await {
        Ok((model, lines)) => {
            for line in lines {
                eprintln!("{line}");
            }
            RunModel::Pinned(model)
        }
        Err(error) => RunModel::Unresolved(error),
    }
}

/// One state, from `--text` or stdin.
async fn decide_text(options: &DecideOptions, target: &Target<'_>) -> Result<()> {
    let state = read_state(options.text.as_deref())?;
    let request = DecisionRequest {
        questions: target.template.questions.clone(),
        context: DecisionContext::Text(state),
    };
    let pinned = match target.model {
        RunModel::Pinned(pinned) => pinned,
        RunModel::Unresolved(error) => {
            if let Some(session) = target.session {
                target
                    .service
                    .record_unresolved(session, &request, error)
                    .await;
            }
            bail!("{}", terminal_advice(&error.to_string()));
        }
    };
    let decisions = match target
        .service
        .decide_pinned(target.session, &pinned.model_ref, pinned.source, &request)
        .await
    {
        Ok(decisions) => decisions,
        Err(error) => {
            // An answer that could not be read was billed all the same.
            report_unrecorded_usage(target, (1, error.billed_usage()));
            bail!("{}", terminal_advice(&error.to_string()));
        }
    };
    if options.json {
        super::print_line_keep_going(serde_json::to_string_pretty(
            &serde_json::json!({ "decisions": decisions }),
        )?);
    } else {
        for decision in &decisions {
            for line in render_decision(decision, Some(target.question)) {
                super::print_line_keep_going(line);
            }
        }
        eprintln!("{PROBABILITY_NOTE}");
    }
    report_unrecorded_usage(target, request_usage(&decisions));
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
                    billed_usage: None,
                })
            }
        }
    }
    let pinned = match target.model {
        RunModel::Pinned(pinned) => pinned,
        RunModel::Unresolved(error) => {
            if let (Some(session), false) = (target.session, chunks.is_empty()) {
                let unsent: Vec<DecisionChunk> =
                    chunks.iter().map(|(_, chunk)| chunk.clone()).collect();
                target
                    .service
                    .record_unresolved_batch(session, &target.template.questions, &unsent, error)
                    .await;
            }
            bail!("{}", terminal_advice(&error.to_string()));
        }
    };
    for group in chunks.chunks(BATCH_GROUP) {
        let positions: Vec<usize> = group.iter().map(|(index, _)| *index).collect();
        let outcomes = target
            .service
            .decide_many_pinned(
                target.session,
                &pinned.model_ref,
                pinned.source,
                &target.template.questions,
                group.iter().map(|(_, chunk)| chunk.clone()).collect(),
            )
            .await
            .map_err(|e| anyhow!("{}", terminal_advice(&e.to_string())))?;
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
        super::print_line_keep_going(serde_json::to_string_pretty(&serde_json::json!({
            "question": options.name,
            "files": files,
            "summary": summary,
        }))?);
    } else {
        for line in render_batch(&rows, target.question) {
            super::print_line_keep_going(line);
        }
        eprintln!("{PROBABILITY_NOTE}");
    }
    report_unrecorded_usage(target, run_usage(&rows));
    Ok(())
}

/// `biorouter models decision list`.
pub async fn handle_models_decision_list(format: &str) -> Result<()> {
    let entries = biorouter::decision::global().models().await;
    if format == "json" {
        super::print_line(serde_json::to_string_pretty(&entries)?);
    } else {
        for line in render_model_list(&entries) {
            super::print_line(line);
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
        // A person reads "failed" as an error: a threshold is passed or not.
        assert_eq!(lines[4], "Threshold: P(yes) 0.31 < 0.80, not passed");
    }

    /// A Baseten decision model is named by its alias wherever a terminal
    /// prints it, as the run's warning and usage line do: the record keeps
    /// the slug, and the Answer line used to print it
    /// (`baseten_decision/decision-models/baseten-joey-2b-latest`).
    #[test]
    fn a_baseten_decision_model_is_named_by_its_alias() {
        let slug = "decision-models/baseten-joey-2b-latest";
        let mut d = yes_no(0.82);
        d.model = DecisionModelRef::new("baseten_decision", slug);
        let lines = render_decision(&d, None);
        let answer = lines.iter().find(|l| l.starts_with("Answer:")).unwrap();
        assert!(answer.contains("  baseten_decision/joey-1  "), "{answer}");
        assert!(!answer.contains("baseten-joey"), "{answer}");
        assert_eq!(d.model.model, slug, "the record keeps the slug");

        let model = DecisionModelRef::new("baseten_decision", slug);
        assert_eq!(short_model(&model), "joey-1");
        assert_eq!(model_label(&model), "baseten_decision/joey-1");
        // An alias is already its own name, and an unknown slug loses only
        // its prefix.
        let alias = DecisionModelRef::new("baseten_decision", "joey-1");
        assert_eq!(model_label(&alias), "baseten_decision/joey-1");
        let unknown = DecisionModelRef::new(
            "baseten_decision",
            "decision-models/baseten-unlisted-99b-latest",
        );
        assert_eq!(short_model(&unknown), "baseten-unlisted-99b-latest");
        assert_eq!(
            model_label(&unknown),
            "baseten_decision/baseten-unlisted-99b-latest"
        );
        // Every other model keeps its id.
        let local = DecisionModelRef::new("ollama_decision", "tev1:0.8b");
        assert_eq!(model_label(&local), "ollama_decision/tev1:0.8b");
        assert_eq!(model_name("ollama_decision", "a/b"), "a/b");
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
                billed_usage: None,
            },
            BatchRow {
                file: "note-0002.txt".into(),
                decision: Some(uncalibrated),
                error: None,
                billed_usage: None,
            },
            BatchRow {
                file: "note-0003.txt".into(),
                decision: Some(pass(0.95)),
                error: None,
                billed_usage: None,
            },
            BatchRow {
                file: "note-0004.txt".into(),
                decision: None,
                error: Some("Chunk `note-0004.txt` is about 20000 tokens".into()),
                billed_usage: None,
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
        assert!(lines[3].contains("no") && lines[3].ends_with("not passed"));
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
                load_refusal: None,
            },
            DecisionModelEntry {
                provider: "baseten_decision".into(),
                model: "decision-models/baseten-elephant-9b-latest".into(),
                display_name: "elephant-1".into(),
                context_tokens: 16_384,
                kinds: vec![DecisionKindTag::Binary],
                tier: ProviderTier::Public,
                affiliation: None,
                adapter: DecisionAdapter::SystemOne,
                local: false,
                load_refusal: None,
            },
        ];
        let lines = render_model_list(&entries);
        assert_eq!(lines[0], "Decision models (2, local first):");
        assert!(lines[1].contains("ollama_decision/tev1:0.8b"));
        assert!(lines[1].contains(
            "private · this computer · 8,192 tokens · yes/no, choice, score · system_one"
        ));
        // R3: where a hosted model runs, beside its tier.
        assert!(lines[2].contains("public · Baseten · 16,384 tokens · yes/no · system_one"));
        // A Baseten model by its alias, the name `--model` takes.
        assert!(
            lines[2].starts_with("  baseten_decision/elephant-1  "),
            "{}",
            lines[2]
        );
        assert!(!lines[2].contains("decision-models/"), "{}", lines[2]);
    }

    /// R3 and privacy: an Ollama cloud tag on a loopback host is served by
    /// ollama.com and is Public, so no line names it "this computer" (the
    /// place used to follow the provider and `OLLAMA_HOST` alone).
    #[test]
    fn an_ollama_cloud_model_never_reads_as_this_computer() {
        assert_eq!(ollama_place("gpt-oss:120b-cloud", true), "Ollama cloud");
        assert_eq!(ollama_place("glm-4.6:cloud", true), "Ollama cloud");
        assert_eq!(ollama_place("tev1:0.8b", true), "this computer");
        assert_eq!(ollama_place("tev1:0.8b", false), "Ollama");
        let cloud = DecisionModelRef::new("ollama_decision", "gpt-oss:120b-cloud");
        assert_eq!(
            model_for_person(&cloud),
            "gpt-oss:120b-cloud · Ollama cloud"
        );
        assert_eq!(
            model_place("ollama_decision", "gpt-oss:120b-cloud"),
            Some("Ollama cloud")
        );
        let line = person_model_names_in_sentence(
            "the chat's default decision model ollama_decision/gpt-oss:120b-cloud could not run.",
        );
        assert_eq!(
            line,
            "the chat's default decision model gpt-oss:120b-cloud (Ollama cloud) could not run."
        );
        let entry = DecisionModelEntry {
            provider: "ollama_decision".into(),
            model: "gpt-oss:120b-cloud".into(),
            display_name: "gpt-oss:120b-cloud".into(),
            context_tokens: 8_192,
            kinds: vec![DecisionKindTag::Binary],
            tier: ProviderTier::Public,
            affiliation: None,
            adapter: DecisionAdapter::SystemOne,
            local: false,
            load_refusal: None,
        };
        let lines = render_model_list(&[entry]);
        assert!(
            lines[1].contains("public · Ollama cloud · 8,192 tokens"),
            "{}",
            lines[1]
        );
        assert!(!lines[1].contains("this computer"), "{}", lines[1]);
    }

    /// R3: a model a person reads (not one they type back) is its alias and
    /// where it runs, and a Baseten slug in a line built elsewhere reads so.
    #[test]
    fn a_model_reads_by_its_alias_and_where_it_runs() {
        let joey =
            DecisionModelRef::new("baseten_decision", "decision-models/baseten-joey-2b-latest");
        assert_eq!(model_for_person(&joey), "joey-1 · Baseten");
        assert_eq!(model_place("baseten_decision", "x"), Some("Baseten"));
        assert_eq!(model_place("versa_azure", "gpt-5.5"), None);
        assert_eq!(
            person_model_names(
                "Default decision model: baseten_decision/decision-models/baseten-joey-2b-latest"
            ),
            "Default decision model: joey-1 · Baseten"
        );
        assert_eq!(
            person_model_names_in_sentence(
                "the chat's default decision model \
                 baseten_decision/decision-models/baseten-joey-2b-latest could not run."
            ),
            "the chat's default decision model joey-1 (Baseten) could not run."
        );
        assert_eq!(
            person_model_names("model baseten_decision/joey-1; and baseten_decision/ alone"),
            "model joey-1 · Baseten; and baseten_decision/ alone"
        );
        assert_eq!(person_model_names("no model here"), "no model here");
        for host in ["localhost", "127.0.0.1", "[::1]", "127.0.0.9"] {
            assert!(host_is_loopback(host), "{host}");
        }
        for host in ["gpu-box", "10.0.0.2", "0.0.0.0"] {
            assert!(!host_is_loopback(host), "{host}");
        }
    }

    /// D-13: a model-only BIOROUTER_DECISION_MODEL is ignored by the
    /// service, so the run says so rather than choosing another model
    /// without a word.
    #[test]
    fn a_default_model_without_a_provider_is_warned_about() {
        let warning = ignored_default_model_warning(None, Some("nonsense")).unwrap();
        assert!(
            warning.contains("BIOROUTER_DECISION_MODEL is nonsense"),
            "{warning}"
        );
        assert!(warning.contains("provider/model"), "{warning}");
        assert_eq!(
            ignored_default_model_warning(None, Some("ollama_decision/tev1:0.8b")),
            None
        );
        assert_eq!(
            ignored_default_model_warning(Some("ollama_decision"), Some("tev1:0.8b")),
            None
        );
        assert_eq!(ignored_default_model_warning(None, None), None);
        assert_eq!(ignored_default_model_warning(None, Some("  ")), None);
    }

    /// R4 and D-9/D-13: a person's refusal names the command, not the
    /// agent's tool; an empty `--ask` and a monitor at 0 are told in the
    /// flag's words.
    #[test]
    fn refusals_read_in_a_terminals_words() {
        assert_eq!(
            terminal_advice("Name a private decision model (decision_models lists them), or …"),
            "Name a private decision model (`biorouter models decision list` shows them), or …"
        );
        let empty = parse(&["--ask", "  ", "--text", "x"]).unwrap();
        assert_eq!(build_question(&empty).unwrap_err().to_string(), EMPTY_ASK);
        assert_eq!(
            parse_positive_probability("0").unwrap_err(),
            "0 is not a probability above 0 and at most 1"
        );
        assert_eq!(
            parse_positive_probability("1.2").unwrap_err(),
            "1.2 is not a probability above 0 and at most 1",
            "one sentence for both ends of the range"
        );
        assert_eq!(parse_positive_probability("0.85"), Ok(0.85));
    }

    #[test]
    fn a_model_ollama_cannot_load_is_listed_with_its_fix() {
        let refusal = biorouter::providers::decision::system_one::ollama_load_refusal_sentence(
            "clef-flash:latest",
        );
        let entry = DecisionModelEntry {
            provider: "ollama_decision".into(),
            model: "clef-flash:latest".into(),
            display_name: "clef-flash:latest".into(),
            context_tokens: 16_384,
            kinds: vec![DecisionKindTag::Binary],
            tier: ProviderTier::Private,
            affiliation: None,
            adapter: DecisionAdapter::SystemOne,
            local: true,
            load_refusal: Some(refusal.clone()),
        };
        let lines = render_model_list(&[entry]);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[1].ends_with("system_one · cannot be loaded"),
            "{}",
            lines[1]
        );
        assert_eq!(lines[2], format!("    {refusal}"));
        assert!(lines[2].contains("ollama pull clef-flash:latest"));
    }

    /// A run with no chat is billed by the provider and recorded nowhere, so
    /// it ends with what it used: per request once, however many questions
    /// the request carried, and the dollars only when the model has a price.
    #[test]
    fn a_run_with_no_chat_says_what_it_used() {
        let usage = |input, output| DecisionUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
        };
        let mut a = yes_no(0.9);
        a.usage = usage(1_000_000, 2);
        let mut a_second_question = a.clone();
        a_second_question.question = "edss".into();
        let mut b = yes_no(0.2);
        b.request_id = "r2".into();
        b.usage = usage(200_000, 1);
        let (requests, total) = request_usage([&a, &a_second_question, &b]);
        assert_eq!(requests, 2);
        assert_eq!(total, usage(1_200_000, 3));

        let mammoth = "baseten_decision/mammoth-1";
        let cost = biorouter::providers::pricing::estimate_cost_usd(
            "baseten_decision",
            "mammoth-1",
            1_200_000,
            3,
        );
        assert_eq!(
            usage_line(mammoth, requests, total, cost).as_deref(),
            Some(
                "usage: 2 requests on baseten_decision/mammoth-1: 1,200,000 input and 3 output \
                 tokens, about $0.12. Without --session this is not recorded in Biorouter's \
                 usage."
            )
        );
        let line = usage_line(
            "baseten_decision/joey-1",
            1,
            usage(120, 1),
            Some(0.000_001_2),
        )
        .unwrap();
        assert!(
            line.starts_with(
                "usage: 1 request on baseten_decision/joey-1: 120 input and 1 output tokens, \
                 less than $0.01."
            ),
            "{line}"
        );
        let local = "ollama_decision/tev1:0.8b";
        let line = usage_line(local, 1, usage(120, 1), None).unwrap();
        assert!(
            line.ends_with(
                "120 input and 1 output tokens. Without --session this is not recorded in \
                 Biorouter's usage."
            ),
            "{line}"
        );
        assert!(!line.contains('$'), "a local model has no price: {line}");
        assert_eq!(usage_line(local, 1, DecisionUsage::default(), None), None);
        let line = usage_line(
            local,
            1,
            DecisionUsage {
                input_tokens: Some(9),
                output_tokens: None,
            },
            None,
        )
        .unwrap();
        assert!(line.contains("9 input and unknown output tokens"), "{line}");
    }

    /// A file whose model answered unreadably was billed, so the run's usage
    /// line counts its request and its tokens beside the decided ones.
    #[test]
    fn a_failed_billed_chunk_counts_in_the_runs_usage_line() {
        let usage = |input, output| DecisionUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
        };
        let mut decided = yes_no(0.9);
        decided.usage = usage(100, 1);
        let rows = vec![
            BatchRow::from_outcome(ChunkOutcome {
                chunk_id: "a.txt".into(),
                decisions: Some(vec![decided]),
                error: None,
                billed_usage: None,
            }),
            BatchRow::from_outcome(ChunkOutcome {
                chunk_id: "b.txt".into(),
                decisions: None,
                error: Some("joey-1 returned an answer that could not be read: x".into()),
                billed_usage: Some(usage(40, 3)),
            }),
            BatchRow::from_outcome(ChunkOutcome {
                chunk_id: "c.txt".into(),
                decisions: None,
                error: Some("refused".into()),
                billed_usage: None,
            }),
        ];
        let (requests, total) = run_usage(&rows);
        assert_eq!((requests, total), (2, usage(140, 4)));
        assert_eq!(
            usage_line("ollama_decision/tev1:0.8b", requests, total, None).as_deref(),
            Some(
                "usage: 2 requests on ollama_decision/tev1:0.8b: 140 input and 4 output tokens. \
                 Without --session this is not recorded in Biorouter's usage."
            )
        );
        // A run where only a failed file was billed still reports it.
        let (requests, total) = run_usage(&rows[1..]);
        assert_eq!((requests, total), (1, usage(40, 3)));
    }

    #[test]
    fn a_run_about_a_chat_says_it_waits_for_late_answers() {
        assert_eq!(
            late_wait_line(2),
            "waiting up to 30 s for 2 decision request(s) that answered late, so their usage is \
             counted"
        );
        assert_eq!(
            late_unsettled_line(1),
            "note: 1 decision request(s) had not answered when this run ended; if the provider \
             bills them, Biorouter does not count it."
        );
    }

    #[test]
    fn threshold_line_for_below_in_both_outcomes() {
        let question = Question {
            instructions: "Is the agent stuck?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Below(Box::new(Threshold::Yes(0.3)))),
        };
        let mut under = yes_no(0.12);
        under.passed = Some(true);
        under.pass_probability = Some(0.12);
        assert_eq!(
            threshold_line(&under, Some(&question)).as_deref(),
            Some("P(yes) 0.12 < 0.30, passed")
        );
        let mut over = yes_no(0.45);
        over.passed = Some(false);
        over.pass_probability = Some(0.45);
        assert_eq!(
            threshold_line(&over, Some(&question)).as_deref(),
            Some("P(yes) 0.45 ≥ 0.30, not passed")
        );
        let score = Question {
            instructions: "How severe?".into(),
            kind: DecisionKind::Score {
                levels: vec!["mild".into(), "moderate".into(), "severe".into()],
            },
            threshold: Some(Threshold::Below(Box::new(Threshold::AtLeastLevel {
                level: 1,
                p: 0.5,
            }))),
        };
        let mut low = yes_no(0.2);
        low.passed = Some(true);
        low.pass_probability = Some(0.2);
        assert_eq!(
            threshold_line(&low, Some(&score)).as_deref(),
            Some("P(at least moderate) 0.20 < 0.50, passed")
        );
    }

    #[test]
    fn no_model_help_names_the_pull_command() {
        assert!(
            NO_MODEL_HELP.contains(biorouter::decision::INSTALL_ADVICE),
            "{NO_MODEL_HELP}"
        );
        assert!(NO_MODEL_HELP.contains("biorouter models decision pull"));
        assert!(!NO_MODEL_HELP.contains("ollama pull"), "{NO_MODEL_HELP}");
    }

    #[test]
    fn the_public_warning_is_one_line_naming_the_model() {
        let line = public_model_warning("elephant-1");
        assert!(!line.contains('\n'));
        assert!(line.starts_with("warning: elephant-1 is hosted outside your institution"));
    }

    /// What a run about a chat says before it sends: a hosted model by name
    /// (unless the private chat's tier refuses it anyway), and, for a chat that
    /// is not private, that the text stays in its decision history.
    #[test]
    fn session_warnings_name_a_public_model_and_a_public_chat_log() {
        let hosted = public_model_warning("joey-1");
        let note = PUBLIC_CHAT_LOG_NOTE.to_string();
        assert_eq!(
            session_warnings(false, true, "joey-1", ProviderTier::Public),
            vec![hosted.clone(), note.clone()]
        );
        assert_eq!(
            session_warnings(false, true, "tev1:0.8b", ProviderTier::Private),
            vec![note]
        );
        assert!(session_warnings(true, true, "tev1:0.8b", ProviderTier::Private).is_empty());
        assert!(
            session_warnings(true, true, "joey-1", ProviderTier::Public).is_empty(),
            "the private chat refuses the hosted model before anything is sent"
        );
        assert_eq!(
            session_warnings(true, false, "joey-1", ProviderTier::Public),
            vec![hosted.clone()],
            "privacy off: the hosted model reads the text, so it is named"
        );
        assert_eq!(
            session_warnings(false, false, "joey-1", ProviderTier::Public),
            vec![hosted]
        );
        assert!(session_warnings(false, false, "tev1:0.8b", ProviderTier::Private).is_empty());
        assert!(!PUBLIC_CHAT_LOG_NOTE.contains('\n'));
    }

    mod pinned {
        use super::*;
        use biorouter::decision::types::{ModelVerdicts, QuestionVerdict};
        use biorouter::decision::{
            DecisionCaps, DecisionError, DecisionModel, DecisionModelSource, DecisionResult,
            DecisionUseGate,
        };
        use biorouter::privacy::{ModelAffiliation, SessionClassification};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;

        fn listed(model: &str, tier: ProviderTier) -> DecisionModelEntry {
            let local = tier == ProviderTier::Private;
            DecisionModelEntry {
                provider: if local {
                    "ollama_decision"
                } else {
                    "baseten_decision"
                }
                .into(),
                model: model.into(),
                display_name: model.into(),
                context_tokens: 4_096,
                kinds: vec![DecisionKindTag::Binary],
                tier,
                affiliation: local.then_some(ModelAffiliation::Local),
                adapter: DecisionAdapter::SystemOne,
                local,
                load_refusal: None,
            }
        }

        /// A local model and a hosted one; the local one can leave the
        /// listing, as it does when Ollama stops answering.
        struct Lab {
            local_gone: Mutex<bool>,
            hosted_calls: Arc<AtomicUsize>,
        }

        struct Answers(DecisionModelEntry, Arc<AtomicUsize>);

        #[async_trait::async_trait]
        impl DecisionModel for Answers {
            fn model_ref(&self) -> DecisionModelRef {
                self.0.model_ref()
            }
            fn display_name(&self) -> String {
                self.0.display_name.clone()
            }
            fn tier(&self) -> ProviderTier {
                self.0.tier
            }
            fn affiliation(&self) -> Option<ModelAffiliation> {
                self.0.affiliation
            }
            fn caps(&self) -> DecisionCaps {
                DecisionCaps {
                    kinds: self.0.kinds.clone(),
                    max_questions: 64,
                    context_tokens: self.0.context_tokens,
                    max_concurrency: 2,
                    max_request_bytes: None,
                    local: self.0.local,
                }
            }
            async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
                self.1.fetch_add(1, Ordering::SeqCst);
                Ok(ModelVerdicts {
                    verdicts: request
                        .questions
                        .keys()
                        .map(|name| QuestionVerdict {
                            question: name.clone(),
                            probabilities: [("yes".to_string(), 0.9), ("no".to_string(), 0.1)]
                                .into_iter()
                                .collect(),
                            source: ProbabilitySource::Native,
                            calibrated: true,
                            reported_choice: None,
                            raw: serde_json::Value::Null,
                        })
                        .collect(),
                    usage: DecisionUsage::default(),
                    served_model: None,
                })
            }
        }

        #[async_trait::async_trait]
        impl DecisionModelSource for Lab {
            async fn list(&self) -> Vec<DecisionModelEntry> {
                let mut entries = Vec::new();
                if !*self.local_gone.lock().unwrap() {
                    entries.push(listed("tev1:0.8b", ProviderTier::Private));
                }
                // Listed as Baseten lists it: by slug, with the alias as its name.
                entries.push(DecisionModelEntry {
                    model: "decision-models/baseten-joey-2b-latest".into(),
                    ..listed("joey-1", ProviderTier::Public)
                });
                entries
            }
            async fn open(
                &self,
                wanted: &DecisionModelRef,
            ) -> DecisionResult<Arc<dyn DecisionModel>> {
                let entry = self
                    .list()
                    .await
                    .into_iter()
                    .find(|e| e.answers_to(wanted))
                    .ok_or_else(|| DecisionError::NoModel("not listed".into()))?;
                let calls = if entry.tier == ProviderTier::Public {
                    self.hosted_calls.clone()
                } else {
                    Arc::new(AtomicUsize::new(0))
                };
                Ok(Arc::new(Answers(entry, calls)))
            }
        }

        struct Open;

        #[async_trait::async_trait]
        impl DecisionUseGate for Open {
            async fn check(
                &self,
                _: &DecisionSession,
                _: &dyn DecisionModel,
            ) -> DecisionResult<()> {
                Ok(())
            }
        }

        fn chunk(id: &str) -> DecisionChunk {
            DecisionChunk {
                id: id.into(),
                context: DecisionContext::Text(format!("SYNTHETIC NOTE {id}")),
            }
        }

        /// `biorouter decide --session` resolves the model once, by the
        /// service's own order, warns about that model, and pins it, so a
        /// later group of a batch fails when the local model leaves the
        /// listing instead of reaching the hosted one.
        #[tokio::test]
        async fn a_run_about_a_chat_pins_the_services_choice() {
            let lab = Arc::new(Lab {
                local_gone: Mutex::new(false),
                hosted_calls: Arc::new(AtomicUsize::new(0)),
            });
            let service = DecisionService::new(lab.clone())
                .with_use_gate(Arc::new(Open))
                .with_default_model(None);
            let public = DecisionSession::new("s", SessionClassification::Public, true);
            let (pinned, lines) = pin_model(&service, Some(&public), None).await.unwrap();
            assert_eq!(
                pinned.model_ref,
                service
                    .resolve_for(Some(&public), None)
                    .await
                    .unwrap()
                    .model_ref(),
                "the terminal chooses what the service would"
            );
            assert_eq!(pinned.model_ref.model, "tev1:0.8b");
            assert_eq!(pinned.label, "ollama_decision/tev1:0.8b");
            assert_eq!(lines, vec![PUBLIC_CHAT_LOG_NOTE.to_string()]);

            let questions = DecisionRequest {
                questions: std::iter::once((
                    "q".to_string(),
                    Question {
                        instructions: "Does the note state MS?".into(),
                        kind: DecisionKind::Binary {},
                        threshold: None,
                    },
                ))
                .collect(),
                context: DecisionContext::Text(String::new()),
            }
            .questions;
            let first = service
                .decide_many(
                    Some(&public),
                    Some(&pinned.model_ref),
                    &questions,
                    vec![chunk("a")],
                )
                .await
                .unwrap();
            assert!(first[0].decisions.is_some());

            *lab.local_gone.lock().unwrap() = true;
            assert!(service
                .decide_many(
                    Some(&public),
                    Some(&pinned.model_ref),
                    &questions,
                    vec![chunk("b")]
                )
                .await
                .is_err());
            assert_eq!(
                lab.hosted_calls.load(Ordering::SeqCst),
                0,
                "nothing moved to the hosted model"
            );

            // A new run now chooses the hosted model, and says so first.
            let (pinned, lines) = pin_model(&service, Some(&public), None).await.unwrap();
            assert_eq!(
                pinned.model_ref.model,
                "decision-models/baseten-joey-2b-latest"
            );
            assert_eq!(
                pinned.label, "baseten_decision/joey-1",
                "the usage line names the model as --model and the docs do"
            );
            assert_eq!(
                lines,
                vec![
                    public_model_warning("joey-1"),
                    PUBLIC_CHAT_LOG_NOTE.to_string()
                ]
            );
            let (alone, lines) = pin_model(&service, None, None).await.unwrap();
            assert_eq!(
                alone.model_ref.model,
                "decision-models/baseten-joey-2b-latest"
            );
            assert_eq!(lines, vec![public_model_warning("joey-1")]);
            // A private chat may use neither: refused before anything is sent.
            let private = DecisionSession::new("p", SessionClassification::Private, true);
            assert!(pin_model(&service, Some(&private), None).await.is_err());
            assert_eq!(lab.hosted_calls.load(Ordering::SeqCst), 0);
        }

        /// Every row the service hands its sink.
        #[derive(Default)]
        struct Rows(Mutex<Vec<biorouter::decision::DecisionLogEntry>>);

        #[async_trait::async_trait]
        impl biorouter::decision::DecisionSink for Rows {
            async fn record(&self, entry: biorouter::decision::DecisionLogEntry) -> Result<()> {
                self.0.lock().unwrap().push(entry);
                Ok(())
            }
        }

        fn options(text: Option<&str>, batch: Option<PathBuf>) -> DecideOptions {
            DecideOptions {
                model: None,
                ask: "Does the note state MS?".into(),
                name: DEFAULT_QUESTION_NAME.into(),
                choices: Vec::new(),
                levels: None,
                threshold: None,
                threshold_on: None,
                text: text.map(str::to_string),
                batch,
                session: Some("p".into()),
                json: true,
            }
        }

        /// `biorouter decide --session` on a private chat that only a hosted
        /// model could answer is refused, and the refusal is logged in the
        /// chat's decision history, as `POST /decisions/decide` logs it. It
        /// used to end before the service saw it, so no row was written.
        #[tokio::test]
        async fn a_refused_run_about_a_chat_is_logged_in_the_chat() {
            let lab = Arc::new(Lab {
                local_gone: Mutex::new(true),
                hosted_calls: Arc::new(AtomicUsize::new(0)),
            });
            let rows = Arc::new(Rows::default());
            let service: &'static DecisionService = Box::leak(Box::new(
                DecisionService::new(lab.clone())
                    .with_use_gate(Arc::new(Open))
                    .with_default_model(None)
                    .with_sink(rows.clone()),
            ));
            let private = DecisionSession::new("p", SessionClassification::Private, true);
            let model = run_model_for_chat(service, &private, None).await;
            assert!(
                matches!(model, RunModel::Unresolved(DecisionError::Refused(_))),
                "{model:?}"
            );
            let single = options(Some("SYNTHETIC NOTE"), None);
            let question = build_question(&single).unwrap();
            let target = Target {
                service,
                session: Some(&private),
                model: &model,
                template: &DecisionRequest {
                    questions: std::iter::once((single.name.clone(), question.clone())).collect(),
                    context: DecisionContext::Text(String::new()),
                },
                question: &question,
            };

            assert!(decide_text(&single, &target).await.is_err());
            {
                let logged = rows.0.lock().unwrap();
                assert_eq!(logged.len(), 1, "one refused row");
                assert_eq!(logged[0].session_id.as_deref(), Some("p"));
                assert_eq!(
                    logged[0].outcome,
                    biorouter::decision::DecisionOutcome::Refused
                );
                assert_eq!(logged[0].context.byte_len(), "SYNTHETIC NOTE".len());
            }

            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.txt"), "SYNTHETIC NOTE A").unwrap();
            std::fs::write(dir.path().join("b.txt"), "SYNTHETIC NOTE B").unwrap();
            let batch = options(None, Some(dir.path().to_path_buf()));
            assert!(decide_directory(&batch, &target, dir.path()).await.is_err());
            let logged = rows.0.lock().unwrap();
            assert_eq!(logged.len(), 2, "one row for the whole batch");
            assert_eq!(
                logged[1].outcome,
                biorouter::decision::DecisionOutcome::Refused
            );
            assert!(logged[1].batch_id.is_some());
            assert_eq!(lab.hosted_calls.load(Ordering::SeqCst), 0, "nothing sent");
        }

        /// A run about a private chat whose model the configured default
        /// chose is refused with the advice to change that default, and one
        /// that named the model with `--model` with the advice to name a
        /// private one, for one text and for a batch alike. Every request names
        /// the pinned model, so the service read every run as `--model` and
        /// told a person who never named a model to name another.
        #[tokio::test]
        async fn a_refusal_advises_by_where_the_pinned_model_came_from() {
            let key_advice = "Set BIOROUTER_DECISION_MODEL to a private model, or start this \
                              work in a public chat.";
            // R4: a terminal names the command that lists the models, never
            // the agent's tool.
            let named_advice = "Name a private decision model (`biorouter models decision \
                                list` shows them), or \
                                start this work in a public chat.";
            let lab = Arc::new(Lab {
                local_gone: Mutex::new(true),
                hosted_calls: Arc::new(AtomicUsize::new(0)),
            });
            let joey = DecisionModelRef::new("baseten_decision", "joey-1");
            let service: &'static DecisionService = Box::leak(Box::new(
                DecisionService::new(lab.clone())
                    .with_use_gate(Arc::new(Open))
                    .with_default_model(Some(joey.clone())),
            ));
            let private = DecisionSession::new("p", SessionClassification::Private, true);
            let single = options(Some("SYNTHETIC NOTE"), None);
            let question = build_question(&single).unwrap();
            let template = DecisionRequest {
                questions: std::iter::once((single.name.clone(), question.clone())).collect(),
                context: DecisionContext::Text(String::new()),
            };
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.txt"), "SYNTHETIC NOTE A").unwrap();
            let batch = options(None, Some(dir.path().to_path_buf()));

            for (wanted, advice) in [(None, key_advice), (Some(joey.clone()), named_advice)] {
                let model = run_model_for_chat(service, &private, wanted.clone()).await;
                let RunModel::Pinned(pinned) = &model else {
                    panic!("the configured default pins: {model:?}");
                };
                assert_eq!(pinned.label, "baseten_decision/joey-1");
                let target = Target {
                    service,
                    session: Some(&private),
                    model: &model,
                    template: &template,
                    question: &question,
                };
                let why = decide_text(&single, &target).await.unwrap_err().to_string();
                assert!(why.ends_with(advice), "{wanted:?}: {why}");

                let outcome = decide_directory(&batch, &target, dir.path()).await;
                let why = outcome.unwrap_err().to_string();
                assert!(why.ends_with(advice), "{wanted:?}, batch: {why}");
            }
            assert_eq!(lab.hosted_calls.load(Ordering::SeqCst), 0, "nothing sent");
        }
    }

    #[test]
    fn small_formatters() {
        assert_eq!(format_latency(22), "22 ms");
        assert_eq!(format_latency(1_000), "1.0 s");
        assert_eq!(
            short_model(&DecisionModelRef::new(
                "baseten_decision",
                "decision-models/baseten-joey-2b-latest"
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
