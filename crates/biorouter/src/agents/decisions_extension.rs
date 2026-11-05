//! The `decisions` platform extension: the `decide` and `decision_models`
//! tools, which let the main model ask a fast decision model typed questions
//! (yes or no, one of named options, a level on a scale) over many pieces of
//! text at once, and get probabilities back instead of prose.
//!
//! Every call goes through [`DecisionService`], so the privacy rule (Gate H: a
//! private chat never reaches a public decision model), the Crew check, the
//! size check and the decision log apply here exactly as they do for a monitor,
//! a phase gate or a route. The master switch is sampled once per tool call
//! (`cap.enforced()`) and threaded into the [`DecisionSession`], never re-read.
//!
//! ⚠ **One tool call is one transcript row, however many decisions it makes.**
//! A batch can be 200 chunks times 16 questions, and a model may call `decide`
//! many times in a turn. The result is therefore an aggregate per question
//! (counts, the highest probability, the ids that passed, capped), never one
//! block per chunk; the decision log is the only place individual calls are
//! listed, and the result names the batch id that finds them there. Small
//! batches (at most [`PER_CHUNK_LINES`] chunks, and few questions) also get one
//! compact line per chunk, because a model conditioning its work on a handful
//! of decisions needs each one's probability.
//!
//! The design builds on Albert Lee's prototype in PR #469, the first decision
//! model integration in BioRouter; the decision-models design (section 5)
//! describes this tool.

use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::Result;
use async_trait::async_trait;
use indexmap::IndexMap;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ProtocolVersion, ServerCapabilities, Tool, ToolAnnotations, ToolsCapability,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
use crate::decision::model::DecisionModelEntry;
use crate::decision::types::{validate_questions, DecisionKindTag};
use crate::decision::{
    Answer, ChunkOutcome, Decision, DecisionAdapter, DecisionChunk, DecisionContext, DecisionKind,
    DecisionModelRef, DecisionOrigin, DecisionService, DecisionSession, Question, Threshold,
};
use crate::privacy::ProviderTier;

pub static EXTENSION_NAME: &str = "decisions";
const EXTENSION_TITLE: &str = "Decisions";

/// The tools as the model calls them: the extension key, `__`, the tool.
pub const DECIDE_TOOL_NAME: &str = "decisions__decide";
pub const DECISION_MODELS_TOOL_NAME: &str = "decisions__decision_models";

/// The decision tools by exact name, for the code-execution filter
/// (`reply_parts::survives_code_execution_filter`), which keeps them directly
/// callable rather than only reachable from a script.
pub fn is_decision_tool_name(tool_name: &str) -> bool {
    tool_name == DECIDE_TOOL_NAME || tool_name == DECISION_MODELS_TOOL_NAME
}

/// Questions in one `decide` call. Under System One's own 64, so one request
/// per chunk stays small and fast.
pub const MAX_TOOL_QUESTIONS: usize = 16;
/// Chunks in one `decide` call. A larger input takes several calls.
pub const MAX_TOOL_CHUNKS: usize = 200;
/// Chunk ids listed per list (passing, uncertain) before the rest are only
/// counted. Fewer per list when a call asks many questions; see [`id_cap`].
pub const LISTED_IDS: usize = 40;
/// The fewest ids a list shows, however many questions a call asks.
const MIN_LISTED_IDS: usize = 10;
/// A small batch also gets one line per chunk, when chunks times questions
/// stays under this.
const PER_CHUNK_CELLS: usize = 80;
/// A batch of at most this many chunks also gets one line per chunk.
pub const PER_CHUNK_LINES: usize = 20;
/// P(yes) in this band counts as uncertain: worth a person's look.
pub const UNCERTAIN_BAND: (f64, f64) = (0.4, 0.6);
/// Distinct error messages listed, and chunk ids listed per message.
const LISTED_ERRORS: usize = 5;
const LISTED_ERROR_IDS: usize = 10;
/// The label every `decide` row carries in the decision log.
pub const DECIDE_ORIGIN_LABEL: &str = "decide";

const DECIDE_DESCRIPTION: &str = concat!(
    "Ask a fast decision model typed questions about many pieces of text at once and get ",
    "probabilities back, not prose. Use it to screen or sort a large input (clinical notes, ",
    "files or functions, document sections, table rows): it answers in tens of milliseconds ",
    "per piece, so do not read hundreds of pieces yourself or call it once per piece.\n",
    "- Split the input along natural boundaries (one note, one function or file, one ",
    "section), each chunk under the model's window (decision_models reports it), each with ",
    "an id you can find it by again.\n",
    "- Put every question for a chunk in ONE call: the questions share the chunk, which is ",
    "read once. Up to 16 questions and 200 chunks per call.\n",
    "- type is yes_no (the default), choice (options: name to meaning) or score (1 to 10 ",
    "levels, lowest first). A threshold marks a pass: P(yes), P(pass_option), or ",
    "P(at_least_level or higher).\n",
    "- A decision model only picks among the answers you offer: it never extracts a value, ",
    "quotes or explains. To get a value, ask whether a chunk states one, then read those ",
    "chunks yourself. Its probabilities can be wrong: read what it flags, and the uncertain ",
    "ones (P(yes) 0.4 to 0.6), before relying on them.\n",
    "Example, 600 neurology notes: questions {\"has_ms\": {\"instructions\": \"Does this note ",
    "state the patient has multiple sclerosis?\", \"threshold\": 0.9}, \"edss_stated\": ",
    "{\"instructions\": \"Is an EDSS score stated in this note?\", \"threshold\": 0.8}} over ",
    "chunks [{\"id\": \"note-0001\", \"text\": \"...\"}] in three calls of 200; then build the ",
    "cohort from has_ms and read only the edss_stated notes to extract the EDSS value.\n",
    "Privacy: a private chat can use only private decision models (on this computer or at ",
    "the user's institution); a public one is refused before anything is sent.\n",
    "The result summarises each question (passes, highest probability, passing ids) under a ",
    "batch id. Every call, with the text it read, is in the chat's decision history."
);

const DECISION_MODELS_DESCRIPTION: &str = concat!(
    "List the decision models `decide` can use: each one's provider/model name, tier ",
    "(private or public), whether it runs on this computer, how it produces probabilities, ",
    "the question types it takes and its window in tokens. A model this chat may not use is ",
    "listed with the reason: a private chat can use only private decision models. Call it ",
    "before choosing a `model` for decide, or to size chunks to the model's window."
);

const INSTRUCTIONS: &str = concat!(
    "Decision models answer typed questions (yes or no, a choice, a score) about a piece ",
    "of text with probabilities, in tens of milliseconds. Use `decide` to screen many ",
    "pieces at once, with every question for a chunk in one call, and `decision_models` ",
    "to see which models this chat may use and how large a chunk each takes."
);

/// `decide`'s arguments.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct DecideParams {
    /// The questions to ask of every chunk, by name (1 to 16). A name is 1 to 64
    /// letters, digits, `_` or `-`, for example "has_ms".
    #[schemars(
        with = "BTreeMap<String, DecideQuestion>",
        extend("minProperties" = 1, "maxProperties" = 16)
    )]
    questions: IndexMap<String, DecideQuestion>,
    /// The pieces to ask about (1 to 200), each with an id you can find it by
    /// again and its text. Split along natural boundaries: one note, one
    /// function or file, one section.
    #[schemars(extend("minItems" = 1, "maxItems" = 200))]
    chunks: Vec<DecideChunk>,
    /// Optional decision model as provider/model, one decision_models lists for
    /// this chat. Leave it out for this chat's default decision model, else the
    /// configured default, else the first model this chat may use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

/// One question as the model writes it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct DecideQuestion {
    /// The question, asked of each chunk on its own, for example "Does this
    /// note state the patient has multiple sclerosis?". `question` is accepted
    /// as another name for it: models reach for that word first.
    #[serde(alias = "question")]
    instructions: String,
    /// yes_no (the default), choice or score.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    #[schemars(extend("enum" = ["yes_no", "choice", "score"]))]
    kind: Option<String>,
    /// For choice: 2 to 26 options, name to what it means, for example
    /// {"rrms": "Relapsing-remitting", "none": "Not stated"}. Offer a "none"
    /// option when the text may not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<BTreeMap<String, String>>")]
    options: Option<IndexMap<String, String>>,
    /// For score: 1 to 10 levels, lowest first, each a short description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    levels: Option<Vec<String>>,
    /// Optional probability from 0 to 1 that marks a pass: P(yes) for yes_no,
    /// P(pass_option) for choice, P(at_least_level or higher) for score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    threshold: Option<f64>,
    /// For choice with a threshold: the option whose probability it is on.
    #[serde(default, alias = "passOption", skip_serializing_if = "Option::is_none")]
    pass_option: Option<String>,
    /// For score with a threshold: the lowest level that passes, numbered from
    /// 0 (the first level).
    #[serde(
        default,
        alias = "atLeastLevel",
        skip_serializing_if = "Option::is_none"
    )]
    at_least_level: Option<usize>,
}

/// One chunk as the model writes it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct DecideChunk {
    /// An id you can find the chunk by again: a note id, a file path, a
    /// section number. Unique in the call.
    id: String,
    /// The chunk's text, exactly as the decision model should read it.
    text: String,
}

/// `decision_models` takes no arguments.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct DecisionModelsParams {}

/// The question types as `decide` names them.
fn kind_word(tag: DecisionKindTag) -> &'static str {
    match tag {
        DecisionKindTag::Binary => "yes_no",
        DecisionKindTag::Choice => "choice",
        DecisionKindTag::Score => "score",
    }
}

fn adapter_words(adapter: DecisionAdapter) -> &'static str {
    match adapter {
        DecisionAdapter::SystemOne => "System One, the model's own probabilities",
        DecisionAdapter::Logprob => "token log-probabilities of a chat model",
        DecisionAdapter::Verbalized => "verbalized: a number the model wrote, never calibrated",
    }
}

fn tier_word(tier: ProviderTier) -> &'static str {
    if tier.is_private() {
        "private"
    } else {
        "public"
    }
}

/// The type a model-written question names, or the one its fields imply.
fn question_type(name: &str, input: &DecideQuestion) -> Result<DecisionKindTag, String> {
    let word = input
        .kind
        .as_deref()
        .map(|k| k.trim().to_ascii_lowercase().replace([' ', '-', '/'], "_"))
        .filter(|k| !k.is_empty());
    Ok(match word.as_deref() {
        None if input.options.is_some() => DecisionKindTag::Choice,
        None if input.levels.is_some() => DecisionKindTag::Score,
        None => DecisionKindTag::Binary,
        Some("yes_no" | "yesno" | "binary" | "boolean" | "bool" | "noul") => {
            DecisionKindTag::Binary
        }
        Some("choice" | "multiple_choice" | "multi_choice" | "options") => DecisionKindTag::Choice,
        Some("score" | "rating" | "scale" | "levels") => DecisionKindTag::Score,
        Some(other) => {
            return Err(format!(
                "Question `{name}`: type `{other}` is not one decide takes. Use yes_no, choice \
                 or score."
            ))
        }
    })
}

/// A threshold that needs a second field to say what it is on (a choice's
/// option, a score's level): both or neither.
fn paired_threshold<T>(
    name: &str,
    threshold: Option<f64>,
    on: Option<T>,
    field: &str,
    what: &str,
) -> Result<Option<(f64, T)>, String> {
    match (threshold, on) {
        (Some(p), Some(on)) => Ok(Some((p, on))),
        (None, None) => Ok(None),
        (Some(_), None) => Err(format!(
            "Question `{name}` has a threshold: say {what} it is on with `{field}`."
        )),
        (None, Some(_)) => Err(format!(
            "Question `{name}` names `{field}` but no `threshold`: give the probability {what} \
             must reach."
        )),
    }
}

fn binary_question(
    name: &str,
    input: &DecideQuestion,
) -> Result<(DecisionKind, Option<Threshold>), String> {
    if input.options.is_some() || input.levels.is_some() {
        return Err(format!(
            "Question `{name}` is yes_no, which takes no options or levels. Set \
             \"type\": \"choice\" or \"score\" if you meant one."
        ));
    }
    if input.pass_option.is_some() || input.at_least_level.is_some() {
        return Err(format!(
            "Question `{name}` is yes_no: its threshold is on P(yes), so it takes no \
             pass_option or at_least_level."
        ));
    }
    Ok((DecisionKind::Binary {}, input.threshold.map(Threshold::Yes)))
}

fn choice_question(
    name: &str,
    input: &DecideQuestion,
) -> Result<(DecisionKind, Option<Threshold>), String> {
    let options = input.options.as_ref().ok_or_else(|| {
        format!("Question `{name}` is a choice, so it needs `options`: name to meaning.")
    })?;
    let options: IndexMap<String, Option<String>> = options
        .iter()
        .map(|(option, meaning)| {
            let meaning = meaning.trim();
            (
                option.trim().to_string(),
                (!meaning.is_empty()).then(|| meaning.to_string()),
            )
        })
        .collect();
    let threshold = paired_threshold(
        name,
        input.threshold,
        input.pass_option.as_deref(),
        "pass_option",
        "which option",
    )?
    .map(|(p, option)| Threshold::Option {
        name: option.trim().to_string(),
        p,
    });
    Ok((DecisionKind::Choice { options }, threshold))
}

fn score_question(
    name: &str,
    input: &DecideQuestion,
) -> Result<(DecisionKind, Option<Threshold>), String> {
    let levels = input.levels.clone().ok_or_else(|| {
        format!("Question `{name}` is a score, so it needs `levels`, lowest first.")
    })?;
    let threshold = paired_threshold(
        name,
        input.threshold,
        input.at_least_level,
        "at_least_level",
        "the lowest passing level (0 is the first)",
    )?
    .map(|(p, level)| Threshold::AtLeastLevel { level, p });
    let levels = levels.into_iter().map(|l| l.trim().to_string()).collect();
    Ok((DecisionKind::Score { levels }, threshold))
}

/// Turn one model-written question into the service's [`Question`].
fn build_question(name: &str, input: &DecideQuestion) -> Result<Question, String> {
    let (kind, threshold) = match question_type(name, input)? {
        DecisionKindTag::Binary => binary_question(name, input)?,
        DecisionKindTag::Choice => choice_question(name, input)?,
        DecisionKindTag::Score => score_question(name, input)?,
    };
    Ok(Question {
        instructions: input.instructions.trim().to_string(),
        kind,
        threshold,
    })
}

/// The service's questions and chunks from `decide`'s arguments, refused with
/// a sentence the model can act on.
fn build_batch(
    params: &DecideParams,
) -> Result<(IndexMap<String, Question>, Vec<DecisionChunk>), String> {
    if params.questions.is_empty() || params.questions.len() > MAX_TOOL_QUESTIONS {
        return Err(format!(
            "Ask between 1 and {MAX_TOOL_QUESTIONS} questions per call, not {}. Every question \
             for a chunk belongs in one call.",
            params.questions.len()
        ));
    }
    if params.chunks.is_empty() || params.chunks.len() > MAX_TOOL_CHUNKS {
        return Err(format!(
            "Send between 1 and {MAX_TOOL_CHUNKS} chunks per call, not {}. Split a larger \
             input over several calls.",
            params.chunks.len()
        ));
    }
    let mut questions = IndexMap::with_capacity(params.questions.len());
    for (name, input) in &params.questions {
        questions.insert(name.trim().to_string(), build_question(name.trim(), input)?);
    }
    validate_questions(&questions)?;

    let mut seen = std::collections::HashSet::new();
    let mut chunks = Vec::with_capacity(params.chunks.len());
    for chunk in &params.chunks {
        let id = chunk.id.trim();
        if id.is_empty() {
            return Err("Every chunk needs a non-empty `id`.".to_string());
        }
        if !seen.insert(id.to_string()) {
            return Err(format!(
                "Chunk id `{id}` appears twice; every chunk needs its own id."
            ));
        }
        if chunk.text.trim().is_empty() {
            return Err(format!(
                "Chunk `{id}` has no text. Drop it, or send the text it stands for."
            ));
        }
        chunks.push(DecisionChunk {
            id: id.to_string(),
            context: DecisionContext::Text(chunk.text.clone()),
        });
    }
    Ok((questions, chunks))
}

/// What a `decide` call returns as structured content: aggregates only, never
/// one entry per chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideSummary {
    /// Always `decide_batch`, so a surface can recognise the shape.
    pub kind: String,
    /// The decision log's `batch_id` (and `origin_id`) for every row of this call.
    pub batch_id: String,
    pub chunks: usize,
    /// Chunks that got an answer.
    pub decided: usize,
    /// Chunks that failed or were refused.
    pub errors: usize,
    pub model: Option<DecisionModelRef>,
    pub tier: Option<ProviderTier>,
    /// False when any answer's probabilities were not calibrated.
    pub calibrated: Option<bool>,
    pub elapsed_ms: u64,
    pub questions: Vec<QuestionSummary>,
    /// The distinct errors, each with the chunks it hit (capped).
    pub error_groups: Vec<ErrorGroup>,
}

/// One question's aggregate over a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionSummary {
    pub name: String,
    /// `yes_no`, `choice` or `score`.
    #[serde(rename = "type")]
    pub kind: String,
    pub threshold: Option<f64>,
    /// What the threshold is on: `yes`, an option name, or `level >= N`.
    pub pass_on: Option<String>,
    /// Chunks that answered this question.
    pub decided: usize,
    /// Chunks that passed the threshold, when there is one.
    pub passed: Option<usize>,
    /// yes_no only: chunks with P(yes) of at least one half.
    pub yes: Option<usize>,
    /// yes_no only: chunks with P(yes) in the uncertain band.
    pub uncertain: Option<usize>,
    /// The highest pass probability (P(yes) for yes_no, else the threshold's
    /// probability, else the most probable answer's).
    pub max_p: Option<f64>,
    /// choice and score: how many chunks gave each answer, in option order.
    pub answer_counts: Option<IndexMap<String, usize>>,
    /// The chunks that passed (or, for yes_no with no threshold, answered yes),
    /// at most [`LISTED_IDS`].
    pub passing_ids: Vec<String>,
    /// How many more passed than are listed.
    pub more_passing: usize,
    /// yes_no only: the uncertain chunks, at most [`LISTED_IDS`].
    pub uncertain_ids: Vec<String>,
    pub more_uncertain: usize,
}

/// One distinct error in a batch and the chunks it hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorGroup {
    pub message: String,
    pub chunk_ids: Vec<String>,
    pub more: usize,
}

fn p_yes(decision: &Decision) -> f64 {
    decision
        .probabilities
        .get(crate::decision::types::YES)
        .copied()
        .unwrap_or(0.0)
}

/// The probability a summary reports for one decision: the threshold's own
/// when there is one, P(yes) for yes_no, else the most probable answer's.
fn reported_p(decision: &Decision) -> f64 {
    if let Some(p) = decision.pass_probability {
        return p;
    }
    match decision.answer {
        Answer::Binary { .. } => p_yes(decision),
        _ => decision
            .probabilities
            .values()
            .copied()
            .fold(0.0_f64, f64::max),
    }
}

fn answer_key(decision: &Decision) -> String {
    match &decision.answer {
        Answer::Binary { value } => if *value { "yes" } else { "no" }.to_string(),
        Answer::Choice { option } => option.clone(),
        Answer::Score { level, label, .. } => {
            if label.is_empty() {
                format!("level {level}")
            } else {
                label.clone()
            }
        }
    }
}

fn pass_on(threshold: Option<&Threshold>) -> Option<String> {
    threshold.map(|t| match t {
        Threshold::Yes(_) => "yes".to_string(),
        Threshold::No(_) => "no".to_string(),
        Threshold::Option { name, .. } => name.clone(),
        Threshold::AtLeastLevel { level, .. } => format!("level >= {level}"),
    })
}

fn threshold_p(threshold: Option<&Threshold>) -> Option<f64> {
    threshold.map(|t| match t {
        Threshold::Yes(p) | Threshold::No(p) => *p,
        Threshold::Option { p, .. } | Threshold::AtLeastLevel { p, .. } => *p,
    })
}

/// Ids listed per list for a call of `questions` questions, so a 16-question
/// call does not list sixteen times as many ids as a one-question call.
pub fn id_cap(questions: usize) -> usize {
    (LISTED_IDS * 3 / questions.max(1)).clamp(MIN_LISTED_IDS, LISTED_IDS)
}

fn capped(ids: Vec<String>, cap: usize) -> (Vec<String>, usize) {
    let more = ids.len().saturating_sub(cap);
    (ids.into_iter().take(cap).collect(), more)
}

/// One question's aggregate over a batch's outcomes.
fn question_summary(
    name: &str,
    question: &Question,
    outcomes: &[ChunkOutcome],
    cap: usize,
) -> QuestionSummary {
    let answered: Vec<(&str, &Decision)> = outcomes
        .iter()
        .filter_map(|o| {
            o.decisions
                .as_ref()?
                .iter()
                .find(|d| d.question == name)
                .map(|d| (o.chunk_id.as_str(), d))
        })
        .collect();
    let binary = matches!(question.kind, DecisionKind::Binary {});
    let has_threshold = question.threshold.is_some();
    let says_yes = |d: &Decision| matches!(d.answer, Answer::Binary { value: true });
    let ids_where = |keep: &dyn Fn(&Decision) -> bool| -> Vec<String> {
        answered
            .iter()
            .filter(|(_, d)| keep(d))
            .map(|(id, _)| id.to_string())
            .collect()
    };
    let passing = ids_where(&|d| {
        if has_threshold {
            d.passed == Some(true)
        } else {
            binary && says_yes(d)
        }
    });
    let uncertain = if binary {
        ids_where(&|d| (UNCERTAIN_BAND.0..=UNCERTAIN_BAND.1).contains(&p_yes(d)))
    } else {
        Vec::new()
    };
    let answer_counts = (!binary).then(|| {
        let mut counts: IndexMap<String, usize> = match &question.kind {
            DecisionKind::Choice { options } => options.keys().map(|k| (k.clone(), 0)).collect(),
            DecisionKind::Score { levels } => levels.iter().map(|l| (l.clone(), 0)).collect(),
            DecisionKind::Binary {} => IndexMap::new(),
        };
        for (_, decision) in &answered {
            *counts.entry(answer_key(decision)).or_default() += 1;
        }
        counts
    });
    let max_p = answered
        .iter()
        .map(|(_, d)| {
            if binary && !has_threshold {
                p_yes(d)
            } else {
                reported_p(d)
            }
        })
        .fold(None, |acc: Option<f64>, p| {
            Some(acc.map_or(p, |a| a.max(p)))
        });
    let passed = has_threshold.then_some(passing.len());
    let yes = binary.then(|| answered.iter().filter(|(_, d)| says_yes(d)).count());
    let uncertain_count = binary.then_some(uncertain.len());
    let (passing_ids, more_passing) = capped(passing, cap);
    let (uncertain_ids, more_uncertain) = capped(uncertain, cap);
    QuestionSummary {
        name: name.to_string(),
        kind: kind_word(question.kind.tag()).to_string(),
        threshold: threshold_p(question.threshold.as_ref()),
        pass_on: pass_on(question.threshold.as_ref()),
        decided: answered.len(),
        passed,
        yes,
        uncertain: uncertain_count,
        max_p,
        answer_counts,
        passing_ids,
        more_passing,
        uncertain_ids,
        more_uncertain,
    }
}

/// The batch's distinct errors, each with the chunks it hit (capped).
fn error_groups(outcomes: &[ChunkOutcome]) -> Vec<ErrorGroup> {
    let mut groups: IndexMap<String, Vec<String>> = IndexMap::new();
    for outcome in outcomes {
        if let Some(error) = &outcome.error {
            groups
                .entry(error.clone())
                .or_default()
                .push(outcome.chunk_id.clone());
        }
    }
    groups
        .into_iter()
        .take(LISTED_ERRORS)
        .map(|(message, ids)| {
            let more = ids.len().saturating_sub(LISTED_ERROR_IDS);
            ErrorGroup {
                message,
                chunk_ids: ids.into_iter().take(LISTED_ERROR_IDS).collect(),
                more,
            }
        })
        .collect()
}

/// The aggregate of one batch. Pure, so the no-flood shape is testable.
pub fn summarize(
    questions: &IndexMap<String, Question>,
    outcomes: &[ChunkOutcome],
    batch_id: &str,
    elapsed_ms: u64,
) -> DecideSummary {
    let decided = outcomes.iter().filter(|o| o.decisions.is_some()).count();
    let first = outcomes
        .iter()
        .find_map(|o| o.decisions.as_ref().and_then(|d| d.first()));
    let calibrated = first.map(|_| {
        outcomes
            .iter()
            .flat_map(|o| o.decisions.iter().flatten())
            .all(|d| d.calibrated)
    });
    DecideSummary {
        kind: "decide_batch".to_string(),
        batch_id: batch_id.to_string(),
        chunks: outcomes.len(),
        decided,
        errors: outcomes.len() - decided,
        model: first.map(|d| d.model.clone()),
        tier: first.map(|d| d.tier),
        calibrated,
        elapsed_ms,
        questions: questions
            .iter()
            .map(|(name, question)| {
                question_summary(name, question, outcomes, id_cap(questions.len()))
            })
            .collect(),
        error_groups: error_groups(outcomes),
    }
}

fn ids_line(ids: &[String], more: usize) -> String {
    let mut line = ids.join(", ");
    if more > 0 {
        line.push_str(&format!(" (and {more} more)"));
    }
    line
}

/// One compact line per chunk, for small batches only.
fn chunk_line(questions: &IndexMap<String, Question>, outcome: &ChunkOutcome) -> String {
    let Some(decisions) = &outcome.decisions else {
        return format!(
            "- {}: failed: {}",
            outcome.chunk_id,
            outcome.error.as_deref().unwrap_or("no answer")
        );
    };
    let parts: Vec<String> = questions
        .keys()
        .filter_map(|name| {
            let decision = decisions.iter().find(|d| &d.question == name)?;
            let verdict = match decision.passed {
                Some(true) => " pass",
                Some(false) => " fail",
                None => "",
            };
            Some(match &decision.answer {
                Answer::Binary { .. } => {
                    format!("{name} P(yes) {:.2}{verdict}", p_yes(decision))
                }
                _ => format!(
                    "{name} {} (P {:.2}){verdict}",
                    answer_key(decision),
                    reported_p(decision)
                ),
            })
        })
        .collect();
    format!("- {}: {}", outcome.chunk_id, parts.join("; "))
}

/// One question's paragraph of the summary text.
fn question_paragraph(question: &QuestionSummary, chunks: usize) -> String {
    let rule = match (&question.pass_on, question.threshold) {
        (Some(on), Some(p)) => format!(", passes at P({on}) >= {p:.2}"),
        _ => String::new(),
    };
    let mut facts: Vec<String> = Vec::new();
    if let Some(passed) = question.passed {
        facts.push(format!("{passed} pass"));
    }
    if let Some(yes) = question.yes {
        facts.push(format!("{yes} yes"));
    }
    if let Some(uncertain) = question.uncertain {
        facts.push(format!(
            "{uncertain} uncertain (P(yes) {:.1} to {:.1})",
            UNCERTAIN_BAND.0, UNCERTAIN_BAND.1
        ));
    }
    if let Some(counts) = &question.answer_counts {
        facts.push(
            counts
                .iter()
                .map(|(answer, n)| format!("{answer} {n}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if let Some(max) = question.max_p {
        facts.push(format!("max P {max:.2}"));
    }
    if question.decided < chunks {
        facts.push(format!("{} answered", question.decided));
    }
    let mut text = format!(
        "\n- {} ({}{rule}): {}.",
        question.name,
        question.kind.replace('_', "/"),
        facts.join("; ")
    );
    if !question.passing_ids.is_empty() {
        let word = if question.passed.is_some() {
            "passed"
        } else {
            "yes"
        };
        text.push_str(&format!(
            "\n  {word}: {}",
            ids_line(&question.passing_ids, question.more_passing)
        ));
    }
    if !question.uncertain_ids.is_empty() {
        text.push_str(&format!(
            "\n  uncertain: {}",
            ids_line(&question.uncertain_ids, question.more_uncertain)
        ));
    }
    text
}

/// The text the model reads: one paragraph per question, never one block per
/// chunk unless the batch is small.
pub fn render_summary(
    summary: &DecideSummary,
    questions: &IndexMap<String, Question>,
    outcomes: &[ChunkOutcome],
) -> String {
    let model = match (&summary.model, summary.tier) {
        (Some(model), Some(tier)) => format!(
            " with {} ({}{})",
            model.label(),
            tier_word(tier),
            match summary.calibrated {
                Some(false) => ", uncalibrated",
                _ => "",
            }
        ),
        _ => String::new(),
    };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut text = format!(
        "decide: {} chunk{} x {} question{}{model} in {:.1} s: {} decided, {} failed. Batch {}.",
        summary.chunks,
        plural(summary.chunks),
        questions.len(),
        plural(questions.len()),
        summary.elapsed_ms as f64 / 1000.0,
        summary.decided,
        summary.errors,
        summary.batch_id,
    );
    for question in &summary.questions {
        text.push_str(&question_paragraph(question, summary.chunks));
    }
    for group in &summary.error_groups {
        let hit = group.chunk_ids.len() + group.more;
        text.push_str(&format!(
            "\nFailed ({hit} chunk{}): {}: {}",
            plural(hit),
            ids_line(&group.chunk_ids, group.more),
            group.message
        ));
    }
    if outcomes.len() <= PER_CHUNK_LINES && outcomes.len() * questions.len() <= PER_CHUNK_CELLS {
        text.push_str("\nPer chunk:");
        for outcome in outcomes {
            text.push('\n');
            text.push_str(&chunk_line(questions, outcome));
        }
    }
    if summary.calibrated == Some(false) {
        text.push_str(
            "\nThese probabilities are uncalibrated (the model wrote them, or most of its \
             probability fell outside the offered answers): treat them as rough.",
        );
    }
    text.push_str(&format!(
        "\nEvery call, with the exact text it read, is in this chat's decision history (batch \
         {}). A decision model only picks among the answers offered and can be wrong: read the \
         chunks it flags before you rely on them.",
        summary.batch_id
    ));
    text
}

/// The `decisions` extension's client.
pub struct DecisionsClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
    service: &'static DecisionService,
}

impl DecisionsClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tasks: None,
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                title: Some(EXTENSION_TITLE.to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(INSTRUCTIONS.to_string()),
        };
        Ok(Self {
            info,
            context,
            service: crate::decision::global(),
        })
    }

    /// Use another decision service (tests: one over a fake model source).
    #[cfg(test)]
    pub(crate) fn with_service(mut self, service: &'static DecisionService) -> Self {
        self.service = service;
        self
    }

    fn tool<T: JsonSchema>(name: &str, description: &str) -> Tool {
        let schema = serde_json::to_value(schema_for!(T)).expect("schema serializes");
        Tool::new(
            name.to_string(),
            description.to_string(),
            schema.as_object().cloned().unwrap_or_default(),
        )
        .annotate(ToolAnnotations {
            title: Some(name.to_string()),
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(true),
        })
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Self::tool::<DecideParams>("decide", DECIDE_DESCRIPTION),
            Self::tool::<DecisionModelsParams>("decision_models", DECISION_MODELS_DESCRIPTION),
        ]
    }

    /// The chat as the decision service sees it, with the master switch as this
    /// call sampled it.
    async fn session(&self, session_id: &str, enforced: bool) -> Result<DecisionSession, String> {
        DecisionSession::load(self.context.session_manager.clone(), session_id, enforced)
            .await
            .map_err(|e| format!("This chat could not be read, so nothing was decided: {e}"))
    }

    /// Resolve a `model` argument against what `decision_models` lists.
    async fn named_model(
        &self,
        session: &DecisionSession,
        wanted: &str,
    ) -> Result<DecisionModelRef, String> {
        let Some(parsed) = DecisionModelRef::parse(wanted) else {
            return Err(format!(
                "`model` is provider/model, for example ollama_decision/tev1:0.8b, not \
                 `{wanted}`. decision_models lists them."
            ));
        };
        let listed = self.service.models_for(Some(session)).await;
        let Some((entry, reason)) = listed.iter().find(|(entry, _)| entry.answers_to(&parsed))
        else {
            let names: Vec<String> = listed
                .iter()
                .filter(|(_, reason)| reason.is_none())
                .map(|(entry, _)| entry.model_ref().label())
                .collect();
            return Err(format!(
                "`{wanted}` is not a decision model this computer offers. Models this chat may \
                 use: {}.",
                if names.is_empty() {
                    "none".to_string()
                } else {
                    names.join(", ")
                }
            ));
        };
        if let Some(reason) = reason {
            return Err(reason.clone());
        }
        Ok(entry.model_ref())
    }

    async fn handle_decide(
        &self,
        session_id: &str,
        enforced: bool,
        arguments: Option<JsonObject>,
        cancel: &CancellationToken,
    ) -> Result<CallToolResult, String> {
        let params: DecideParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .map_err(|e| format!("decide's arguments do not fit its schema: {e}"))?;
        let (questions, chunks) = build_batch(&params)?;

        let batch_id = uuid::Uuid::now_v7().to_string();
        let session = self
            .session(session_id, enforced)
            .await?
            .with_origin(DecisionOrigin::Tool, Some(batch_id.clone()))
            .with_origin_label(Some(DECIDE_ORIGIN_LABEL.to_string()))
            .with_turn(crate::decision::turns::current_turn(session_id));
        let model = match params.model.as_deref().map(str::trim) {
            Some(wanted) if !wanted.is_empty() => Some(self.named_model(&session, wanted).await?),
            // The chat's own default, checked as a named model is, so a default
            // this chat may not use is refused with its reason up front.
            _ => match &session.chat_default_model {
                Ok(Some(default)) => Some(
                    self.named_model(&session, &default.label())
                        .await
                        .map_err(|why| {
                            format!("This chat's default decision model is not usable: {why}")
                        })?,
                ),
                Ok(None) => None,
                Err(why) => {
                    return Err(format!(
                        "{why}, so this chat's default decision model is unknown. Name one in \
                         `model`."
                    ))
                }
            },
        };

        let started = Instant::now();
        let run = self.service.decide_batch(
            Some(&session),
            model.as_ref(),
            &questions,
            chunks,
            &batch_id,
        );
        let outcomes = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(format!(
                    "The decide call was cancelled. Chunks already answered are in the \
                     decision history (batch {batch_id})."
                ));
            }
            result = run => result.map_err(|e| e.to_string())?,
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let summary = summarize(&questions, &outcomes, &batch_id, elapsed_ms);
        let text = render_summary(&summary, &questions, &outcomes);
        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: serde_json::to_value(&summary).ok(),
            // Nothing was decided: say so as an error, so neither the model nor
            // the tool card reads an all-failed batch as an answer.
            is_error: Some(summary.decided == 0),
            meta: None,
        })
    }

    async fn handle_decision_models(
        &self,
        session_id: &str,
        enforced: bool,
    ) -> Result<CallToolResult, String> {
        let session = self.session(session_id, enforced).await?;
        let crew_scoped = match crate::crew::manager() {
            Ok(manager) => manager.is_scoped_session(session_id).await,
            Err(_) => true,
        };
        let listed = self.service.models_for(Some(&session)).await;
        let rows: Vec<ModelRow> = listed
            .into_iter()
            .map(|(entry, reason)| {
                let reason = if crew_scoped {
                    Some(
                        "This chat works under a Crew grant, which fixes the one model its \
                         content may reach."
                            .to_string(),
                    )
                } else {
                    reason
                };
                ModelRow::from_entry(&entry, reason)
            })
            .collect();
        let private_chat = enforced && session.classification.is_private();
        let text = render_models(&rows, private_chat);
        let structured = serde_json::json!({
            "kind": "decision_models",
            "chatPrivate": private_chat,
            "models": rows,
        });
        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(structured),
            is_error: Some(false),
            meta: None,
        })
    }
}

/// One decision model as `decision_models` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    /// provider/model, the spelling decide's `model` takes.
    pub name: String,
    pub provider: String,
    pub model: String,
    pub display_name: String,
    pub tier: ProviderTier,
    pub local: bool,
    pub adapter: DecisionAdapter,
    /// Whether its probabilities are calibrated: yes for System One, no for
    /// verbalized, and unknown ahead of time for log-probabilities (it depends
    /// on how much probability lands on the offered answers).
    pub calibrated: Option<bool>,
    pub kinds: Vec<String>,
    pub context_tokens: usize,
    pub available: bool,
    pub unavailable_reason: Option<String>,
}

impl ModelRow {
    fn from_entry(entry: &DecisionModelEntry, reason: Option<String>) -> Self {
        Self {
            name: entry.model_ref().label(),
            provider: entry.provider.clone(),
            model: entry.model.clone(),
            display_name: entry.display_name.clone(),
            tier: entry.tier,
            local: entry.local,
            adapter: entry.adapter,
            calibrated: match entry.adapter {
                DecisionAdapter::SystemOne => Some(true),
                DecisionAdapter::Verbalized => Some(false),
                DecisionAdapter::Logprob => None,
            },
            kinds: entry
                .kinds
                .iter()
                .map(|k| kind_word(*k).to_string())
                .collect(),
            context_tokens: entry.context_tokens,
            available: reason.is_none(),
            unavailable_reason: reason,
        }
    }
}

/// The models as the model reads them: those this chat may use first.
pub fn render_models(rows: &[ModelRow], private_chat: bool) -> String {
    if rows.is_empty() {
        return "No decision model is available. Pull one into a local Ollama (for example \
                `ollama pull tev1:0.8b`), add a Baseten API key, or set \
                BIOROUTER_DECISION_MODEL."
            .to_string();
    }
    let mut text = if private_chat {
        "This chat is private: only private decision models (ones that run on this computer \
         or the user's institution) can read it, and a public one is refused before anything \
         is sent."
            .to_string()
    } else {
        "Decision models:".to_string()
    };
    let (available, unavailable): (Vec<&ModelRow>, Vec<&ModelRow>) =
        rows.iter().partition(|row| row.available);
    for row in available.iter().chain(unavailable.iter()) {
        let name = if row.display_name != row.model {
            format!("{} ({})", row.name, row.display_name)
        } else {
            row.name.clone()
        };
        text.push_str(&format!(
            "\n- {name}: {}{}, {}; takes {}; window about {} tokens.",
            tier_word(row.tier),
            if row.local {
                ", runs on this computer"
            } else {
                ""
            },
            adapter_words(row.adapter),
            row.kinds.join(", "),
            row.context_tokens
        ));
        if let Some(reason) = &row.unavailable_reason {
            text.push_str(&format!(" Not available to this chat: {reason}"));
        }
    }
    if available.is_empty() {
        text.push_str("\nNone of these may read this chat, so decide will be refused here.");
    } else {
        text.push_str(
            "\nName one in decide's `model` as provider/model, or leave `model` out for this \
             chat's default decision model, else the configured default, else the first model \
             this chat may use.",
        );
    }
    text
}

#[async_trait]
impl McpClientTrait for DecisionsClient {
    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        meta: McpMeta,
        cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        // Sampled once for this call and threaded: never re-read below.
        let enforced = meta.capability.enforced();
        let session_id = meta.session_id.as_str();
        let result = match name {
            "decide" => {
                self.handle_decide(session_id, enforced, arguments, &cancellation_token)
                    .await
            }
            "decision_models" => self.handle_decision_models(session_id, enforced).await,
            other => Err(format!("Unknown tool: {other}")),
        };
        Ok(result.unwrap_or_else(|error| {
            CallToolResult::error(vec![Content::text(format!("Error: {error}"))])
        }))
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

/// A fake decision model source for tests here and in the phase-gate tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use indexmap::IndexMap;

    use crate::decision::error::{DecisionError, DecisionResult};
    use crate::decision::model::{
        DecisionCaps, DecisionModel, DecisionModelEntry, DecisionModelSource,
    };
    use crate::decision::types::{
        DecisionKindTag, ModelVerdicts, ProbabilitySource, QuestionVerdict, NO, YES,
    };
    use crate::decision::{
        DecisionAdapter, DecisionKind, DecisionModelRef, DecisionRequest, DecisionService,
        DecisionSession, DecisionUsage, DecisionUseGate,
    };
    use crate::privacy::ProviderTier;

    /// How the fake answers. Text that contains `FAIL` errors; otherwise a
    /// state that contains `[state-p=0.95]` gets that P(yes) for every
    /// question, a question whose instructions contain `[p=0.42]` gets P(yes) =
    /// 0.42 (or that much on its first option), and every other question
    /// `default_p`.
    #[derive(Clone)]
    pub(crate) struct FakeModel {
        pub(crate) entry: DecisionModelEntry,
        pub(crate) default_p: f64,
        pub(crate) calls: Arc<AtomicUsize>,
        pub(crate) fail_all: bool,
    }

    fn marked_p(text: &str, mark: &str) -> Option<f64> {
        text.split(mark)
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .and_then(|p| p.parse().ok())
    }

    #[async_trait]
    impl DecisionModel for FakeModel {
        fn model_ref(&self) -> DecisionModelRef {
            self.entry.model_ref()
        }
        fn display_name(&self) -> String {
            self.entry.display_name.clone()
        }
        fn tier(&self) -> ProviderTier {
            self.entry.tier
        }
        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            self.entry.affiliation
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: self.entry.kinds.clone(),
                max_questions: 64,
                context_tokens: self.entry.context_tokens,
                max_concurrency: 4,
                max_request_bytes: None,
                local: self.entry.local,
            }
        }
        async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_all || request.context.render_text().contains("FAIL") {
                return Err(DecisionError::Backend {
                    model: self.entry.model.clone(),
                    status: Some(500),
                    error_type: None,
                    message: "the decision server failed".into(),
                });
            }
            let verbalized = matches!(self.entry.adapter, DecisionAdapter::Verbalized);
            let state_p = marked_p(&request.context.render_text(), "[state-p=");
            let verdicts = request
                .questions
                .iter()
                .map(|(name, question)| {
                    let p = state_p
                        .or_else(|| marked_p(&question.instructions, "[p="))
                        .unwrap_or(self.default_p);
                    let keys = question.kind.outcome_keys();
                    let probabilities: IndexMap<String, f64> = match &question.kind {
                        DecisionKind::Binary {} => {
                            IndexMap::from([(YES.to_string(), p), (NO.to_string(), 1.0 - p)])
                        }
                        _ => {
                            let rest = (1.0 - p) / (keys.len() - 1).max(1) as f64;
                            keys.iter()
                                .enumerate()
                                .map(|(i, k)| (k.clone(), if i == 0 { p } else { rest }))
                                .collect()
                        }
                    };
                    QuestionVerdict {
                        question: name.clone(),
                        probabilities,
                        source: if verbalized {
                            ProbabilitySource::Verbalized { reason: None }
                        } else {
                            ProbabilitySource::Native
                        },
                        calibrated: !verbalized,
                        reported_choice: None,
                        raw: serde_json::json!({}),
                    }
                })
                .collect();
            Ok(ModelVerdicts {
                verdicts,
                usage: DecisionUsage {
                    input_tokens: Some(12),
                    output_tokens: Some(1),
                },
                served_model: None,
            })
        }
    }

    pub(crate) struct FakeSource {
        pub(crate) models: Vec<FakeModel>,
    }

    #[async_trait]
    impl DecisionModelSource for FakeSource {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            self.models.iter().map(|m| m.entry.clone()).collect()
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            self.models
                .iter()
                .find(|m| m.entry.answers_to(wanted))
                .cloned()
                .map(|m| Arc::new(m) as Arc<dyn DecisionModel>)
                .ok_or_else(|| DecisionError::NoModel(format!("no model {}", wanted.label())))
        }
    }

    struct Allow;

    #[async_trait]
    impl DecisionUseGate for Allow {
        async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
            Ok(())
        }
    }

    pub(crate) fn entry(
        name: &str,
        tier: ProviderTier,
        local: bool,
        adapter: DecisionAdapter,
    ) -> DecisionModelEntry {
        DecisionModelEntry {
            provider: "test_decision".into(),
            model: name.into(),
            display_name: name.into(),
            context_tokens: 4_096,
            kinds: vec![
                DecisionKindTag::Binary,
                DecisionKindTag::Choice,
                DecisionKindTag::Score,
            ],
            tier,
            affiliation: local.then_some(crate::privacy::affiliation::ModelAffiliation::Local),
            adapter,
            local,
        }
    }

    pub(crate) fn model(entry: DecisionModelEntry, default_p: f64) -> FakeModel {
        FakeModel {
            entry,
            default_p,
            calls: Arc::new(AtomicUsize::new(0)),
            fail_all: false,
        }
    }

    /// A service over `models` that never reads config, never asks Crew and
    /// lives for the rest of the test process.
    pub(crate) fn service(models: Vec<FakeModel>) -> &'static DecisionService {
        service_with_default(models, None)
    }

    /// [`service`] with `default` standing in for `BIOROUTER_DECISION_MODEL`.
    pub(crate) fn service_with_default(
        models: Vec<FakeModel>,
        default: Option<DecisionModelRef>,
    ) -> &'static DecisionService {
        Box::leak(Box::new(
            DecisionService::new(Arc::new(FakeSource { models }))
                .with_use_gate(Arc::new(Allow))
                .with_default_model(default),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use super::test_support::{entry, model, service};
    use super::*;
    use crate::privacy::{CallCapability, SessionClassification};
    use crate::session::{SessionManager, SessionType};
    use serde_json::json;

    async fn client_with(
        models: Vec<test_support::FakeModel>,
        private: bool,
    ) -> (DecisionsClient, String) {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        std::mem::forget(temp);
        let session = manager
            .create_session(std::env::temp_dir(), "decide".into(), SessionType::User)
            .await
            .unwrap();
        if private {
            manager
                .update(&session.id)
                .raise_privacy(SessionClassification::Private, "test:private")
                .apply()
                .await
                .unwrap();
        }
        let client = DecisionsClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: manager,
        })
        .unwrap()
        .with_service(service(models));
        (client, session.id)
    }

    async fn call(
        client: &DecisionsClient,
        session: &str,
        tool: &str,
        args: serde_json::Value,
        cap: CallCapability,
    ) -> CallToolResult {
        let args: JsonObject = serde_json::from_value(args).unwrap();
        client
            .call_tool(
                tool,
                Some(args),
                McpMeta::new(session, cap),
                CancellationToken::new(),
            )
            .await
            .unwrap()
    }

    fn text(result: &CallToolResult) -> String {
        result.content[0].as_text().unwrap().text.clone()
    }

    fn enforced() -> CallCapability {
        CallCapability::for_test(ProviderTier::Private, true)
    }

    fn notes(n: usize) -> Vec<serde_json::Value> {
        (0..n)
            .map(|i| {
                json!({
                    "id": format!("note-{i:04}"),
                    "text": if i % 50 == 7 { "FAIL this note".to_string() } else { format!("Neuro follow-up {i}") }
                })
            })
            .collect()
    }

    #[test]
    fn the_extension_offers_exactly_decide_and_decision_models() {
        let names: Vec<String> = DecisionsClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names, ["decide", "decision_models"]);
        assert_eq!(DECIDE_TOOL_NAME, format!("{EXTENSION_NAME}__decide"));
        assert_eq!(
            DECISION_MODELS_TOOL_NAME,
            format!("{EXTENSION_NAME}__decision_models")
        );
    }

    /// The description is the only place the model learns how to use the
    /// tool well, so the rules the owner asked for are pinned here.
    #[test]
    fn the_decide_description_teaches_chunking_one_call_per_chunk_and_privacy() {
        for needle in [
            "natural boundaries",
            "every question for a chunk in ONE call",
            "only picks among the answers you offer",
            "multiple sclerosis",
            "EDSS",
            "a private chat can use only private decision models",
            "decision history",
        ] {
            assert!(DECIDE_DESCRIPTION.contains(needle), "missing {needle:?}");
        }
    }

    /// Measured live: GPT-5.5 wrote `{"type": "yes_no", "question": "…"}` twice
    /// before reading the module, and both calls failed on the missing
    /// `instructions`. The word it reaches for is accepted.
    #[test]
    fn a_question_written_as_question_is_read_as_its_instructions() {
        let q: DecideQuestion = serde_json::from_value(json!({
            "type": "yes_no",
            "question": "Does this note state an EDSS score?",
            "threshold": 0.8
        }))
        .unwrap();
        assert_eq!(q.instructions, "Does this note state an EDSS score?");
        assert!(is_decision_tool_name(DECIDE_TOOL_NAME));
        assert!(is_decision_tool_name(DECISION_MODELS_TOOL_NAME));
        assert!(!is_decision_tool_name("decisions__other"));
    }

    #[test]
    fn questions_are_built_by_type_and_bad_shapes_are_refused_with_a_reason() {
        let q = |v: serde_json::Value| serde_json::from_value::<DecideQuestion>(v).unwrap();
        let yes_no = build_question(
            "has_ms",
            &q(json!({"instructions": "MS?", "threshold": 0.9})),
        )
        .unwrap();
        assert_eq!(yes_no.kind, DecisionKind::Binary {});
        assert_eq!(yes_no.threshold, Some(Threshold::Yes(0.9)));

        let choice = build_question(
            "course",
            &q(
                json!({"instructions": "Course?", "options": {"rrms": "Relapsing", "none": ""},
                       "threshold": 0.7, "passOption": "rrms"}),
            ),
        )
        .unwrap();
        let DecisionKind::Choice { options } = &choice.kind else {
            panic!("{choice:?}")
        };
        assert_eq!(
            options.get("none"),
            Some(&None),
            "an empty meaning sends the name"
        );
        assert_eq!(
            choice.threshold,
            Some(Threshold::Option {
                name: "rrms".into(),
                p: 0.7
            })
        );

        let score = build_question(
            "severity",
            &q(json!({"instructions": "How severe?", "type": "score",
                       "levels": ["mild", "moderate", "severe"], "threshold": 0.6, "at_least_level": 1})),
        )
        .unwrap();
        assert_eq!(
            score.threshold,
            Some(Threshold::AtLeastLevel { level: 1, p: 0.6 })
        );

        for (value, expected) in [
            (
                json!({"instructions": "x", "type": "essay"}),
                "not one decide takes",
            ),
            (
                json!({"instructions": "x", "type": "choice"}),
                "needs `options`",
            ),
            (
                json!({"instructions": "x", "options": {"a": "", "b": ""}, "threshold": 0.5}),
                "pass_option",
            ),
            (
                json!({"instructions": "x", "levels": ["a"], "threshold": 0.5}),
                "at_least_level",
            ),
            (
                json!({"instructions": "x", "pass_option": "a"}),
                "takes no pass_option",
            ),
        ] {
            let err = build_question("q", &q(value)).unwrap_err();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }
    }

    #[test]
    fn a_batch_refuses_too_many_questions_or_chunks_and_empty_or_repeated_chunks() {
        let params = |questions: usize, chunks: Vec<serde_json::Value>| {
            let questions: serde_json::Map<String, serde_json::Value> = (0..questions)
                .map(|i| (format!("q{i}"), json!({"instructions": "yes?"})))
                .collect();
            serde_json::from_value::<DecideParams>(
                json!({"questions": questions, "chunks": chunks}),
            )
            .unwrap()
        };
        let one = vec![json!({"id": "a", "text": "t"})];
        assert!(build_batch(&params(16, one.clone())).is_ok());
        assert!(build_batch(&params(17, one.clone()))
            .unwrap_err()
            .contains("between 1 and 16"));
        assert!(build_batch(&params(0, one.clone())).is_err());
        assert!(build_batch(&params(1, notes(201)))
            .unwrap_err()
            .contains("between 1 and 200"));
        assert!(build_batch(&params(1, vec![]))
            .unwrap_err()
            .contains("between 1 and 200"));
        let repeated = vec![
            json!({"id": "a", "text": "t"}),
            json!({"id": "a", "text": "u"}),
        ];
        assert!(build_batch(&params(1, repeated))
            .unwrap_err()
            .contains("appears twice"));
        let empty = vec![json!({"id": "a", "text": "  "})];
        assert!(build_batch(&params(1, empty))
            .unwrap_err()
            .contains("no text"));
        let bad_name = serde_json::from_value::<DecideParams>(json!({
            "questions": {"has ms": {"instructions": "x"}}, "chunks": one
        }))
        .unwrap();
        assert!(build_batch(&bad_name).unwrap_err().contains("has ms"));
    }

    /// The owner rule: a big batch is ONE compact result. Per question it
    /// carries counts, the highest p and capped passing ids; it never lists
    /// every chunk.
    #[tokio::test]
    async fn a_big_batch_is_one_aggregate_result_and_never_one_block_per_chunk() {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.95,
        );
        let calls = local.calls.clone();
        let (client, session) = client_with(vec![local], true).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({
                "questions": {
                    "has_ms": {"instructions": "Does this note state MS?", "threshold": 0.9},
                    "edss_stated": {"instructions": "Is an EDSS score stated? [p=0.5]", "threshold": 0.8},
                    "course": {"instructions": "Which course? [p=0.7]", "type": "choice",
                               "options": {"rrms": "Relapsing", "spms": "Secondary", "none": "Not stated"}}
                },
                "chunks": notes(200),
            }),
            enforced(),
        )
        .await;
        assert_eq!(result.is_error, Some(false), "{}", text(&result));
        assert_eq!(calls.load(Ordering::SeqCst), 200, "one request per chunk");

        let body = text(&result);
        assert!(
            !body.contains("note-0100"),
            "a big batch must not list every chunk: {body}"
        );
        assert!(!body.contains("Per chunk"), "{body}");
        assert!(body.contains("196 decided, 4 failed"), "{body}");
        assert!(body.contains("has_ms"), "{body}");
        assert!(body.contains("196 pass"), "{body}");
        assert!(body.contains("(and 156 more)"), "{body}");
        assert!(
            body.contains("196 uncertain"),
            "edss at 0.5 is uncertain: {body}"
        );
        assert!(body.contains("rrms 196, spms 0, none 0"), "{body}");
        assert!(body.contains("decision history"), "{body}");
        assert!(
            body.len() < 12_000,
            "the result stays compact: {}",
            body.len()
        );

        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(summary.kind, "decide_batch");
        assert!(body.contains(&summary.batch_id));
        assert_eq!(summary.chunks, 200);
        assert_eq!(summary.errors, 4);
        assert_eq!(summary.error_groups.len(), 1);
        assert_eq!(summary.error_groups[0].chunk_ids.len(), 4);
        let has_ms = &summary.questions[0];
        assert_eq!(has_ms.passed, Some(196));
        assert_eq!(has_ms.passing_ids.len(), LISTED_IDS);
        assert_eq!(has_ms.more_passing, 196 - LISTED_IDS);
        assert_eq!(summary.tier, Some(ProviderTier::Private));
        assert_eq!(summary.calibrated, Some(true));
    }

    #[tokio::test]
    async fn a_small_batch_also_gets_one_line_per_chunk() {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.2,
        );
        let (client, session) = client_with(vec![local], false).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({
                "questions": {"done": {"instructions": "Is it done?", "threshold": 0.5}},
                "chunks": [{"id": "a", "text": "one"}, {"id": "b", "text": "FAIL two"}],
            }),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert!(body.contains("Per chunk:"), "{body}");
        assert!(body.contains("- a: done P(yes) 0.20 fail"), "{body}");
        assert!(body.contains("- b: failed:"), "{body}");
    }

    /// Gate H on the tool path: nothing is sent when a private chat names, or
    /// can only find, a public model.
    #[tokio::test]
    async fn a_private_chat_never_reaches_a_public_decision_model() {
        let cloud = model(
            entry(
                "cloud",
                ProviderTier::Public,
                false,
                DecisionAdapter::SystemOne,
            ),
            0.9,
        );
        let calls = cloud.calls.clone();
        let (client, session) = client_with(vec![cloud], true).await;
        let args = |model: Option<&str>| {
            let mut args = json!({
                "questions": {"q": {"instructions": "yes?"}},
                "chunks": [{"id": "a", "text": "private note"}],
            });
            if let Some(model) = model {
                args["model"] = json!(model);
            }
            args
        };
        let named = call(
            &client,
            &session,
            "decide",
            args(Some("test_decision/cloud")),
            enforced(),
        )
        .await;
        assert_eq!(named.is_error, Some(true));
        assert!(text(&named).contains("private"), "{}", text(&named));
        let automatic = call(&client, &session, "decide", args(None), enforced()).await;
        assert_eq!(automatic.is_error, Some(true));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");

        // The master switch as this call sampled it decides: off, it is sent.
        let off = call(
            &client,
            &session,
            "decide",
            args(None),
            CallCapability::for_test(ProviderTier::Private, false),
        )
        .await;
        assert_eq!(off.is_error, Some(false), "{}", text(&off));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// With no `model`, decide asks the chat's own default decision model
    /// (checked as a named model is), not the configured default.
    #[tokio::test]
    async fn decide_without_a_model_uses_the_chats_default_decision_model() {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.9,
        );
        let cloud = model(
            entry(
                "cloud",
                ProviderTier::Public,
                false,
                DecisionAdapter::SystemOne,
            ),
            0.9,
        );
        let (local_calls, cloud_calls) = (local.calls.clone(), cloud.calls.clone());
        let (client, session) = client_with(vec![], false).await;
        let client = client.with_service(test_support::service_with_default(
            vec![local, cloud],
            Some(DecisionModelRef::new("test_decision", "cloud")),
        ));
        let args = json!({
            "questions": {"q": {"instructions": "yes?"}},
            "chunks": [{"id": "a", "text": "a note"}],
        });

        let configured = call(&client, &session, "decide", args.clone(), enforced()).await;
        assert_eq!(configured.is_error, Some(false), "{}", text(&configured));
        assert_eq!(cloud_calls.load(Ordering::SeqCst), 1);

        crate::decision::settings::save(
            &client.context.session_manager,
            &session,
            crate::decision::settings::DecisionSettings {
                default_model: Some(DecisionModelRef::new("test_decision", "tev1")),
                monitors: Vec::new(),
            },
        )
        .await
        .unwrap();
        let chosen = call(&client, &session, "decide", args.clone(), enforced()).await;
        assert_eq!(chosen.is_error, Some(false), "{}", text(&chosen));
        assert_eq!(local_calls.load(Ordering::SeqCst), 1, "the chat's default");
        assert_eq!(
            cloud_calls.load(Ordering::SeqCst),
            1,
            "not the configured one"
        );

        crate::decision::settings::save(
            &client.context.session_manager,
            &session,
            crate::decision::settings::DecisionSettings {
                default_model: Some(DecisionModelRef::new("test_decision", "gone")),
                monitors: Vec::new(),
            },
        )
        .await
        .unwrap();
        let missing = call(&client, &session, "decide", args, enforced()).await;
        assert_eq!(missing.is_error, Some(true));
        assert!(
            text(&missing).contains("default decision model is not usable"),
            "{}",
            text(&missing)
        );
        assert_eq!(
            local_calls.load(Ordering::SeqCst) + cloud_calls.load(Ordering::SeqCst),
            2,
            "nothing else was sent"
        );
    }

    #[tokio::test]
    async fn a_model_must_be_one_decision_models_lists() {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.9,
        );
        let (client, session) = client_with(vec![local], false).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({
                "questions": {"q": {"instructions": "yes?"}},
                "chunks": [{"id": "a", "text": "x"}],
                "model": "test_decision/gpt-giant",
            }),
            enforced(),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let body = text(&result);
        assert!(body.contains("test_decision/tev1"), "{body}");
        let unparsable = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": [{"id": "a", "text": "x"}], "model": "tev1"}),
            enforced(),
        )
        .await;
        assert!(text(&unparsable).contains("provider/model"));
    }

    #[tokio::test]
    async fn decision_models_says_why_a_model_is_unavailable_to_this_chat() {
        let (client, session) = client_with(
            vec![
                model(
                    entry(
                        "cloud",
                        ProviderTier::Public,
                        false,
                        DecisionAdapter::SystemOne,
                    ),
                    0.9,
                ),
                model(
                    entry(
                        "tev1",
                        ProviderTier::Private,
                        true,
                        DecisionAdapter::SystemOne,
                    ),
                    0.9,
                ),
            ],
            true,
        )
        .await;
        let result = call(&client, &session, "decision_models", json!({}), enforced()).await;
        assert_eq!(result.is_error, Some(false));
        let body = text(&result);
        assert!(body.starts_with("This chat is private"), "{body}");
        let tev1 = body.find("test_decision/tev1").unwrap();
        let cloud = body.find("test_decision/cloud").unwrap();
        assert!(tev1 < cloud, "usable models come first: {body}");
        assert!(body.contains("Not available to this chat"), "{body}");
        let rows: Vec<ModelRow> =
            serde_json::from_value(result.structured_content.unwrap()["models"].clone()).unwrap();
        let cloud = rows.iter().find(|r| r.model == "cloud").unwrap();
        assert!(!cloud.available && cloud.unavailable_reason.is_some());
        let tev1 = rows.iter().find(|r| r.model == "tev1").unwrap();
        assert!(tev1.available && tev1.local);
        assert_eq!(tev1.kinds, ["yes_no", "choice", "score"]);
    }

    #[test]
    fn a_call_with_many_questions_lists_fewer_ids_per_question() {
        assert_eq!(id_cap(1), LISTED_IDS);
        assert_eq!(id_cap(3), LISTED_IDS);
        assert_eq!(id_cap(4), 30);
        assert_eq!(id_cap(MAX_TOOL_QUESTIONS), 10);
    }

    /// A batch in which nothing was decided is an error, so neither the model
    /// nor the tool card reads it as an answer.
    #[tokio::test]
    async fn a_batch_that_decided_nothing_is_an_error() {
        let mut broken = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.9,
        );
        broken.fail_all = true;
        let (client, session) = client_with(vec![broken], false).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({
                "questions": {"q": {"instructions": "yes?"}},
                "chunks": [{"id": "a", "text": "x"}, {"id": "b", "text": "y"}],
            }),
            enforced(),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let body = text(&result);
        assert!(body.contains("0 decided, 2 failed"), "{body}");
        assert!(body.contains("Failed (2 chunks): a, b"), "{body}");
    }

    #[test]
    fn an_empty_catalog_says_how_to_get_a_decision_model() {
        assert!(render_models(&[], false).contains("ollama pull tev1:0.8b"));
    }
}
