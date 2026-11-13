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
//! (counts, the highest probability, the ids that passed and the ids to review,
//! capped), never one block per chunk; the decision log is the only place
//! individual calls are listed, and the result names the batch id that finds
//! them there. Small batches (at most [`PER_CHUNK_LINES`] chunks, and few
//! questions) also get one compact line per chunk, because a model conditioning
//! its work on a handful of decisions needs each one's probability.
//!
//! A chunk that is neither a clear no nor a clear pass ([`needs_review`]) is
//! listed under review, and a per-chunk verdict never says a bare "fail":
//! measured, an agent read "fail" as a no and reported a true RRMS note at
//! P(yes) 0.36 as not having multiple sclerosis.
//!
//! The design builds on Albert Lee's prototype in PR #469, the first decision
//! model integration in BioRouter; the decision-models design (section 5)
//! describes this tool.

use std::collections::BTreeMap;
use std::time::Instant;

mod vote;
pub use vote::{VoteChunk, VoteModel, VoteQuestion, VoteSummary};

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
use crate::decision::service::{usable_chunk, ModelSource, UsableChunk};
use crate::decision::types::MAX_CHOICE_OPTIONS;
use crate::decision::types::{validate_questions, DecisionKindTag};
use crate::decision::{
    model_in_words, Answer, ChunkOutcome, Decision, DecisionAdapter, DecisionChunk,
    DecisionContext, DecisionKind, DecisionModelRef, DecisionOrigin, DecisionService,
    DecisionSession, Question, Threshold,
};
use crate::privacy::ProviderTier;
use crate::providers::decision::catalog::{
    choice_option_limit, decision_model_display_label, decision_model_short_name,
    OLLAMA_DECISION_PROVIDER,
};
use crate::providers::decision::system_one::OLLAMA_MAX_REQUEST_BYTES;

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
/// Chunk ids listed per list (passing, to review) before the rest are only
/// counted. Fewer per list when a call asks many questions; see [`id_cap`].
pub const LISTED_IDS: usize = 40;
/// The fewest ids a list shows, however many questions a call asks.
const MIN_LISTED_IDS: usize = 10;
/// A small batch also gets one line per chunk, when chunks times questions
/// stays under this.
const PER_CHUNK_CELLS: usize = 80;
/// A batch of at most this many chunks also gets one line per chunk.
pub const PER_CHUNK_LINES: usize = 20;
/// Below this pass probability a chunk is a clear no and is not listed for
/// review. Measured on tev1:0.8b, true multiple sclerosis notes that only used
/// an abbreviation (RRMS, SPMS, PPMS) scored 0.34 to 0.53 on a bare question,
/// and clear negatives scored 0.10 or less, so a floor of 0.2 lists the near
/// misses and leaves the clear negatives out.
pub const REVIEW_FLOOR: f64 = 0.2;
/// At or above this pass probability (and the threshold, when it is higher) a
/// chunk is a clear pass and is not listed for review.
pub const CLEAR_PASS: f64 = 0.8;
/// Under this probability of its top answer (the most probable of the answers
/// offered, whatever the threshold is on), an answer is low confidence, and
/// its chunk is listed for a second look, when `escalate` sets no other bound.
pub const LOW_CONFIDENCE: f64 = 0.8;

/// Whether a chunk whose pass probability is `p` must be read before its
/// answer is reported: it is not a clear no (`p` at least [`REVIEW_FLOOR`])
/// and not a clear pass (`p` under the larger of the threshold and
/// [`CLEAR_PASS`]). The band is fixed on purpose: the agent that has to read
/// the list does not get to choose how long it is.
pub fn needs_review(p: f64, threshold: Option<f64>) -> bool {
    p >= REVIEW_FLOOR && p < review_ceiling(threshold)
}

/// The top of the review band: the larger of the threshold and
/// [`CLEAR_PASS`], or [`CLEAR_PASS`] with no threshold.
fn review_ceiling(threshold: Option<f64>) -> f64 {
    threshold.map_or(CLEAR_PASS, |t| t.max(CLEAR_PASS))
}
/// Distinct error messages listed, and chunk ids listed per message.
const LISTED_ERRORS: usize = 5;
const LISTED_ERROR_IDS: usize = 10;
/// The label every `decide` row carries in the decision log.
pub const DECIDE_ORIGIN_LABEL: &str = "decide";
/// What a cancelled `decide` call says.
const CANCELLED: &str =
    "The decide call was cancelled. Chunks already answered are in the decision history.";

const DECIDE_DESCRIPTION: &str = concat!(
    "Ask a fast decision model typed questions about many pieces of text at once and get ",
    "probabilities back, not prose. Use it to screen or sort a large input (clinical notes, ",
    "files or functions, document sections, table rows): it answers in tens of milliseconds ",
    "per piece, so do not read hundreds of pieces yourself or call it once per piece.\n",
    "- Split the input along natural boundaries (one note, one function or file, one ",
    "section), each chunk within the size decision_models gives for the model (in tokens and ",
    "in characters of prose or of numbers, which take far more tokens; it also says which ",
    "model is used when `model` is left out), each with an id you can find it by again.\n",
    "- Put every question for a chunk in ONE call: the questions share the chunk, which is ",
    "read once. Up to 16 questions and 200 chunks per call.\n",
    "- type is yes_no (the default), choice (options: name to meaning) or score (1 to 10 ",
    "levels, lowest first). A threshold marks a pass: P(yes), P(pass_option), or ",
    "P(at_least_level or higher).\n",
    "- A decision model only picks among the answers you offer: it never extracts a value, ",
    "quotes or explains. To get a value, ask whether a chunk states one, then read those ",
    "chunks yourself. Its probabilities can be wrong: read the chunks the result lists under ",
    "review before you report them.\n",
    "- For a screen, spell out abbreviations in the question and make it a choice whose ",
    "options yes and no each say what they mean in one sentence, with a threshold near 0.5 ",
    "and pass_option yes (has_ms below). A higher threshold fails true cases; the review list ",
    "catches the rest.\n",
    "- To have another model re-check only the unsure answers in the same call, add escalate ",
    "{\"model\": \"<another decision model>\", \"below\": 0.8}.\n",
    "Example, 600 neurology notes: questions {\"has_ms\": {\"type\": \"choice\", ",
    "\"instructions\": \"Does this note state the patient has multiple sclerosis (MS, RRMS, ",
    "SPMS or PPMS)?\", \"options\": {\"yes\": \"The patient has multiple sclerosis (MS, RRMS, ",
    "SPMS or PPMS).\", \"no\": \"The patient does not have multiple sclerosis.\"}, ",
    "\"threshold\": 0.5, \"pass_option\": \"yes\"}, \"edss_stated\": {\"instructions\": \"Is an ",
    "EDSS (Expanded Disability Status Scale) score stated in this note?\", \"threshold\": 0.5}} ",
    "over chunks [{\"id\": \"note-0001\", \"text\": \"...\"}] in three calls of 200; then build ",
    "the cohort from the has_ms passes after reading its review list, and read only the ",
    "edss_stated notes to extract the EDSS value.\n",
    "Privacy: a private chat can use only private decision models (on this computer or at ",
    "the user's institution), for escalate too; a public one is refused before anything is ",
    "sent.\n",
    "The result summarises each question (passes, highest probability, passing ids, ids to ",
    "review) and lists every low-confidence id. Every call, with the text it read, is in the ",
    "chat's decision history."
);

const DECISION_MODELS_DESCRIPTION: &str = concat!(
    "List the decision models `decide` can use: each one's name (the one `model` takes), tier ",
    "(private or public), whether it runs on this computer, how it produces probabilities, ",
    "the question types it takes, its window in tokens and how much text one chunk can carry ",
    "on it (in characters of prose and of numbers), and which model decide uses here when ",
    "`model` is left out. A model this chat may not use is listed with the reason: a private ",
    "chat can use only private decision models. Call it before choosing a `model` for decide, ",
    "and to size chunks."
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
    /// Optional decision model as provider/model, or by its name or alias when
    /// that names one model decision_models lists for this chat (`joey-1`).
    /// Leave it out for this chat's default decision model, else the
    /// configured default, else the first model this chat may use;
    /// decision_models says which that is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Instead of model, 2 to 5 models named the same way, for a vote: each
    /// chunk is asked of every one, and the result gives each model's answer
    /// and the majority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minItems" = 2, "maxItems" = 5))]
    models: Option<Vec<String>>,
    /// Optional second opinion in the same call: after the first model
    /// decides every chunk, each answer whose top probability is under
    /// `below` is asked again of this model, and its answer is final. Not
    /// with `models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    escalate: Option<DecideEscalate>,
    /// Optional: decide only the chunks with these ids, out of `chunks`, for
    /// example the low-confidence ids an earlier result listed.
    #[serde(default, alias = "onlyIds", skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minItems" = 1, "maxItems" = 200))]
    only_ids: Option<Vec<String>>,
}

/// `decide`'s `escalate`: the model that answers again where the first was
/// unsure, and how unsure counts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct DecideEscalate {
    /// The second model, named as `model` takes it, for example
    /// "clef-flash:latest". Not the first model.
    model: String,
    /// Optional, above 0 and at most 1 (default 0.8): an answer whose top
    /// probability is under this is asked again.
    #[serde(default, alias = "threshold", skip_serializing_if = "Option::is_none")]
    below: Option<f64>,
}

/// One question as the model writes it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct DecideQuestion {
    /// The question, asked of each chunk on its own, with its abbreviations
    /// spelled out, for example "Does this note state the patient has multiple
    /// sclerosis (MS, RRMS, SPMS or PPMS)?". `question` is accepted as another
    /// name for it: models reach for that word first.
    #[serde(alias = "question")]
    instructions: String,
    /// yes_no (the default), choice or score.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    #[schemars(extend("enum" = ["yes_no", "choice", "score"]))]
    kind: Option<String>,
    /// For choice: 2 to 26 options (some models take fewer: decision_models
    /// says), name to what it means, for example
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
        DecisionAdapter::SystemOne => {
            "System One, the model's own probabilities, not validated on your data"
        }
        DecisionAdapter::Logprob => {
            "token log-probabilities of a chat model, not validated on your data"
        }
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
fn build_batch(params: &DecideParams) -> Result<(IndexMap<String, Question>, CallChunks), String> {
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
    Ok((questions, only_chunks(chunks, params.only_ids.as_deref())?))
}

/// A call's chunks to decide, each with its place in the call's `chunks`
/// (from 0), which the decision history numbers it by. With `only_ids` some
/// places are skipped, so a second pass names a chunk by the number the
/// first gave it.
type CallChunks = Vec<(usize, DecisionChunk)>;

/// The chunks `only_ids` names, in `chunks` order, or every chunk when it is
/// left out. An id that is not a chunk of the call, or is named twice, is
/// refused, so a second pass never silently decides fewer chunks than asked.
fn only_chunks(chunks: Vec<DecisionChunk>, only: Option<&[String]>) -> Result<CallChunks, String> {
    let chunks = chunks.into_iter().enumerate();
    let Some(only) = only else {
        return Ok(chunks.collect());
    };
    let chunks: CallChunks = chunks.collect();
    let mut wanted = std::collections::HashSet::with_capacity(only.len());
    for id in only {
        let id = id.trim();
        if !chunks.iter().any(|(_, chunk)| chunk.id == id) {
            return Err(format!(
                "`only_ids` names `{id}`, which is not the id of a chunk in this call. Send that \
                 chunk in `chunks`, or leave its id out."
            ));
        }
        if !wanted.insert(id.to_string()) {
            return Err(format!(
                "`only_ids` names `{id}` twice; name each chunk once."
            ));
        }
    }
    if wanted.is_empty() {
        return Err("`only_ids` names no chunk. Name at least one, or leave it out.".to_string());
    }
    Ok(chunks
        .into_iter()
        .filter(|(_, chunk)| wanted.contains(&chunk.id))
        .collect())
}

/// `escalate`, checked: the model's name as written, and the bound.
#[derive(Debug, Clone, PartialEq)]
struct EscalationAsk {
    model: String,
    below: f64,
}

/// `escalate`, when the call gives it: a model name, a bound above 0 and at
/// most 1 (0.8 when left out), and never with a vote.
fn escalation_ask(params: &DecideParams) -> Result<Option<EscalationAsk>, String> {
    let Some(escalate) = &params.escalate else {
        return Ok(None);
    };
    if params.models.is_some() {
        return Err(
            "`escalate` asks a second model after one first model, so it does not go with a vote \
             in `models`. Use one or the other."
                .to_string(),
        );
    }
    let model = escalate.model.trim();
    if model.is_empty() {
        return Err(
            "`escalate` needs `model`: the decision model to ask again, named as `model` takes it."
                .to_string(),
        );
    }
    let below = escalate.below.unwrap_or(LOW_CONFIDENCE);
    if !below.is_finite() || below <= 0.0 || below > 1.0 {
        return Err(format!(
            "`escalate`'s `below` is a probability above 0 and at most 1, not {below}."
        ));
    }
    Ok(Some(EscalationAsk {
        model: model.to_string(),
        below,
    }))
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
    /// The probability under which a top answer is low confidence:
    /// `escalate`'s `below`, else [`LOW_CONFIDENCE`].
    pub low_confidence_below: f64,
    /// Every chunk, in call order and never capped, with a final answer whose
    /// top probability is under `low_confidence_below`: the ids to decide
    /// again (`only_ids` with another `model`) or to read. After an
    /// escalation, the ones still unsure.
    pub low_confidence_ids: Vec<String>,
    /// What `escalate` did, when the call asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<EscalationSummary>,
}

/// What `escalate` did: which chunks the second model was asked about, and
/// its answers beside the first model's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EscalationSummary {
    /// The model asked again.
    pub model: DecisionModelRef,
    /// Its name as a person reads it (an alias for a Baseten model).
    pub label: String,
    pub tier: Option<ProviderTier>,
    /// The first model's name as a person reads it.
    pub first_label: String,
    /// A question whose top answer was less probable than this was asked again.
    pub below: f64,
    /// Chunks the first model decided.
    pub first_decided: usize,
    /// Chunks escalated: each had at least one answer under `below`, to be
    /// asked again (those too large for the second model never were; see
    /// `too_large_ids`).
    pub escalated: usize,
    /// Answers escalated (a chunk with two unsure answers counts two).
    pub answers: usize,
    /// Of those, answers the second model gave.
    pub decided: usize,
    /// Of those, answers that differ from the first model's.
    pub changed: usize,
    /// Answers escalated that the second model gave no answer to, so the
    /// first model's answer stands: it failed, or the chunk was too large
    /// for it. `answers` is `decided` plus `errors`.
    pub errors: usize,
    /// Every chunk escalated, in call order (at most [`MAX_TOOL_CHUNKS`]).
    pub items: Vec<EscalatedItem>,
    /// The second model's distinct errors, each with the chunks it hit; a
    /// chunk too large for it is not among them.
    pub error_groups: Vec<ErrorGroup>,
    /// Every escalated chunk too large for the second model's window beside
    /// the questions asked again, in call order and never capped: it was not
    /// sent to that model, and the first model's answers stand for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub too_large_ids: Vec<String>,
}

/// One chunk asked again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EscalatedItem {
    pub chunk_id: String,
    /// The questions asked again, in question order.
    pub answers: Vec<EscalatedAnswer>,
}

/// One answer asked again: the final answer, which model gave it, and the
/// first model's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EscalatedAnswer {
    pub question: String,
    /// The final answer and the probability of it.
    pub label: String,
    pub p: f64,
    /// The name of the model that gave the final answer: the second model's,
    /// or the first's when the second failed.
    pub by: String,
    pub first_label: String,
    pub first_p: f64,
    /// Why the second model gave no answer, when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// True when the chunk was too large for the second model's window, so
    /// it was never sent to it; `error` says how large.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub too_large: bool,
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
    /// Chunks to read before their answer is reported (see [`needs_review`]):
    /// for yes_no on P(yes), or on the threshold's probability when there is
    /// one; for choice and score only with a threshold. `None` when the rule
    /// does not apply (a choice or score with no threshold).
    pub review: Option<usize>,
    /// The highest pass probability (P(yes) for yes_no, else the threshold's
    /// probability, else the most probable answer's).
    pub max_p: Option<f64>,
    /// choice and score: how many chunks gave each answer, in option order.
    pub answer_counts: Option<IndexMap<String, usize>>,
    /// choice and score: per answer, in option order, the chunks that gave
    /// it, at most [`LISTED_IDS`] each (the rest are counted in
    /// `answer_counts`), so a label can be reported per chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_ids: Option<IndexMap<String, Vec<String>>>,
    /// The chunks that passed (or, for yes_no with no threshold, answered yes),
    /// at most [`LISTED_IDS`].
    pub passing_ids: Vec<String>,
    /// How many more passed than are listed.
    pub more_passing: usize,
    /// The chunks to review, at most [`LISTED_IDS`].
    pub review_ids: Vec<String>,
    /// How many more are to review than are listed.
    pub more_review: usize,
    /// The answered chunks split into four groups that do not overlap and add
    /// up to `decided`, for a question with a pass (a threshold, or a yes_no).
    /// `None` for a choice or score with no threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<AnswerGroups>,
}

/// G-3: a question's answered chunks in four groups that do not overlap, so
/// their counts add up. "yes" is a pass (P at or above the threshold; P(yes) at
/// least one half for a yes_no with no threshold), "no" is everything else; a
/// yes is confident at `confident_from` and up, a no under `review_from`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerGroups {
    pub yes_confident: usize,
    pub yes_review: usize,
    pub no_review: usize,
    pub no_confident: usize,
    /// Where a yes starts: the threshold, or one half for a yes_no with none.
    pub pass_from: f64,
    /// Where a yes becomes confident: the larger of the threshold and 0.8.
    pub confident_from: f64,
    /// Where a no stops being confident: 0.2.
    pub review_from: f64,
}

/// Which of the four [`AnswerGroups`] one decision falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    YesConfident,
    YesReview,
    NoReview,
    NoConfident,
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

/// The answer a threshold is about. A `below` threshold is about its inner
/// threshold's answer.
fn pass_on(threshold: Option<&Threshold>) -> Option<String> {
    threshold.map(|t| match t {
        Threshold::Yes(_) => "yes".to_string(),
        Threshold::No(_) => "no".to_string(),
        Threshold::Option { name, .. } => name.clone(),
        Threshold::AtLeastLevel { level, .. } => format!("level >= {level}"),
        Threshold::Below(inner) => pass_on(Some(inner.as_ref())).unwrap_or_default(),
    })
}

/// The bound a threshold compares against (its inner threshold's for a
/// `below`).
fn threshold_p(threshold: Option<&Threshold>) -> Option<f64> {
    threshold.map(Threshold::probability)
}

/// The probability the review rule reads for one decision: the threshold's
/// own when the question has one (P(yes), P(pass_option), P(at_least_level
/// or higher)), P(yes) for a yes_no with no threshold, and `None` for a choice
/// or score with no threshold, which has no pass to review against.
fn review_probability(question: &Question, decision: &Decision) -> Option<f64> {
    match (&question.threshold, &question.kind) {
        (Some(_), _) => Some(
            decision
                .pass_probability
                .unwrap_or_else(|| reported_p(decision)),
        ),
        (None, DecisionKind::Binary {}) => Some(p_yes(decision)),
        (None, _) => None,
    }
}

/// Which group a decision falls in, for a question with a pass: its side from
/// whether it passed (or, for a yes_no with no threshold, answered yes), and its
/// confidence from the review rule's probability. `None` for a choice or score
/// with no threshold.
fn group_of(question: &Question, decision: &Decision) -> Option<Group> {
    let p = review_probability(question, decision)?;
    let yes = if question.threshold.is_some() {
        decision.passed == Some(true)
    } else {
        matches!(decision.answer, Answer::Binary { value: true })
    };
    let ceiling = review_ceiling(threshold_p(question.threshold.as_ref()));
    Some(match (yes, p) {
        (true, p) if p >= ceiling => Group::YesConfident,
        (true, _) => Group::YesReview,
        (false, p) if p >= REVIEW_FLOOR => Group::NoReview,
        (false, _) => Group::NoConfident,
    })
}

/// Whether this decision is on the review list: a yes short of a confident
/// one or a no that is not a clear no. With a threshold of at least
/// [`REVIEW_FLOOR`] this is exactly [`needs_review`]; under it, a weak pass is
/// listed too, so the review list is always the two "to review" groups.
fn in_review(question: &Question, decision: &Decision) -> bool {
    matches!(
        group_of(question, decision),
        Some(Group::YesReview | Group::NoReview)
    )
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

/// How a reviewed question's answers fall into the four confidence groups.
fn answer_groups<'a>(
    question: &Question,
    decisions: impl Iterator<Item = &'a Decision>,
) -> AnswerGroups {
    let mut groups = AnswerGroups {
        yes_confident: 0,
        yes_review: 0,
        no_review: 0,
        no_confident: 0,
        pass_from: threshold_p(question.threshold.as_ref()).unwrap_or(0.5),
        confident_from: review_ceiling(threshold_p(question.threshold.as_ref())),
        review_from: REVIEW_FLOOR,
    };
    for decision in decisions {
        match group_of(question, decision) {
            Some(Group::YesConfident) => groups.yes_confident += 1,
            Some(Group::YesReview) => groups.yes_review += 1,
            Some(Group::NoReview) => groups.no_review += 1,
            Some(Group::NoConfident) | None => groups.no_confident += 1,
        }
    }
    groups
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
    // A yes_no always has a pass to review against (P(yes) itself); a choice
    // or score has one only through its threshold.
    let reviewed = binary || has_threshold;
    let review = ids_where(&|d| in_review(question, d));
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
    let answer_ids = answer_counts.as_ref().map(|counts| {
        let mut ids: IndexMap<String, Vec<String>> = counts
            .keys()
            .map(|answer| (answer.clone(), Vec::new()))
            .collect();
        for (id, decision) in &answered {
            let listed = ids.entry(answer_key(decision)).or_default();
            if listed.len() < cap {
                listed.push(id.to_string());
            }
        }
        ids
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
    let groups = reviewed.then(|| answer_groups(question, answered.iter().map(|(_, d)| *d)));
    let passed = has_threshold.then_some(passing.len());
    let yes = binary.then(|| answered.iter().filter(|(_, d)| says_yes(d)).count());
    let review_count = reviewed.then_some(review.len());
    let (passing_ids, more_passing) = capped(passing, cap);
    let (review_ids, more_review) = capped(review, cap);
    QuestionSummary {
        name: name.to_string(),
        kind: kind_word(question.kind.tag()).to_string(),
        threshold: threshold_p(question.threshold.as_ref()),
        pass_on: pass_on(question.threshold.as_ref()),
        decided: answered.len(),
        passed,
        yes,
        review: review_count,
        max_p,
        answer_counts,
        answer_ids,
        passing_ids,
        more_passing,
        review_ids,
        more_review,
        groups,
    }
}

/// What two errors share when they differ only in the chunk they name and
/// its numbers (an over-size refusal names the chunk and its size): the
/// message with the chunk's id and every run of digits taken out.
fn error_shape(message: &str, chunk_id: &str) -> String {
    let mut shape = String::with_capacity(message.len());
    let mut in_digits = false;
    for ch in message.replace(&format!("`{chunk_id}`"), "`…`").chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                shape.push('#');
            }
            in_digits = true;
        } else {
            shape.push(ch);
            in_digits = false;
        }
    }
    shape
}

/// The batch's distinct errors, each with the chunks it hit (capped). Errors
/// that differ only in the chunk they name and its numbers are one group,
/// shown by its first message: 50 oversized chunks are one group of 50, not
/// 50 groups of which five were listed. Past [`LISTED_ERRORS`] groups, the
/// rest are one last group, so every failed chunk is counted.
fn error_groups(outcomes: &[ChunkOutcome]) -> Vec<ErrorGroup> {
    let mut groups: IndexMap<String, (String, Vec<String>)> = IndexMap::new();
    for outcome in outcomes {
        if let Some(error) = &outcome.error {
            groups
                .entry(error_shape(error, &outcome.chunk_id))
                .or_insert_with(|| (error.clone(), Vec::new()))
                .1
                .push(outcome.chunk_id.clone());
        }
    }
    let group = |message: String, ids: Vec<String>| {
        let more = ids.len().saturating_sub(LISTED_ERROR_IDS);
        ErrorGroup {
            message,
            chunk_ids: ids.into_iter().take(LISTED_ERROR_IDS).collect(),
            more,
        }
    };
    let mut groups = groups.into_values();
    let mut listed: Vec<ErrorGroup> = groups
        .by_ref()
        .take(LISTED_ERRORS)
        .map(|(message, ids)| group(message, ids))
        .collect();
    let rest: Vec<(String, Vec<String>)> = groups.collect();
    if !rest.is_empty() {
        let kinds = rest.len();
        let ids = rest.into_iter().flat_map(|(_, ids)| ids).collect();
        listed.push(group(
            format!(
                "{kinds} other error{}; the chat's decision history has each one.",
                if kinds == 1 { "" } else { "s" }
            ),
            ids,
        ));
    }
    listed
}

/// The probability of a decision's top answer: the most probable of the
/// answers offered, whatever a threshold is on.
fn top_p(decision: &Decision) -> f64 {
    decision
        .probabilities
        .values()
        .copied()
        .fold(0.0_f64, f64::max)
}

/// Whether a decision's top answer is less probable than `below`.
fn unsure(decision: &Decision, below: f64) -> bool {
    top_p(decision) < below
}

/// Every decided chunk, in call order, with an answer whose top probability
/// is under `below`. Never capped: an agent that decides exactly these again
/// (`only_ids`) must be able to name every one.
fn low_confidence_ids(outcomes: &[ChunkOutcome], below: f64) -> Vec<String> {
    outcomes
        .iter()
        .filter(|outcome| {
            outcome
                .decisions
                .iter()
                .flatten()
                .any(|decision| unsure(decision, below))
        })
        .map(|outcome| outcome.chunk_id.clone())
        .collect()
}

/// The aggregate of one batch. Pure, so the no-flood shape is testable.
/// `low_below` is the bound under which a top answer is low confidence.
pub fn summarize(
    questions: &IndexMap<String, Question>,
    outcomes: &[ChunkOutcome],
    batch_id: &str,
    elapsed_ms: u64,
    low_below: f64,
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
        low_confidence_below: low_below,
        low_confidence_ids: low_confidence_ids(outcomes, low_below),
        escalation: None,
    }
}

/// The name a result gives a decision model: its bare name or alias
/// (`tev1:0.8b`, `joey-1`), which `model` also takes.
fn model_label(model: &DecisionModelRef) -> String {
    decision_model_short_name(model)
}

/// What the escalation stage was asked and answered, before it is merged.
struct EscalationRun {
    model: DecisionModelRef,
    tier: Option<ProviderTier>,
    below: f64,
    /// The second model's outcome for each chunk escalated, in call order,
    /// each with only the questions asked again. A chunk too large for it
    /// has an outcome with no answer and the sentence that says how large.
    outcomes: Vec<ChunkOutcome>,
    /// The escalated chunks too large for the second model's window beside
    /// the questions they would be asked again, never sent to it.
    too_large: std::collections::HashSet<String>,
}

/// Per decided chunk, in call order, the questions whose top answer is less
/// probable than `below`: what `escalate` asks the second model again.
fn escalation_targets(
    questions: &IndexMap<String, Question>,
    outcomes: &[ChunkOutcome],
    below: f64,
) -> Vec<(String, Vec<String>)> {
    outcomes
        .iter()
        .filter_map(|outcome| {
            let decisions = outcome.decisions.as_ref()?;
            let names: Vec<String> = questions
                .keys()
                .filter(|name| {
                    decisions
                        .iter()
                        .any(|d| &d.question == *name && unsure(d, below))
                })
                .cloned()
                .collect();
            (!names.is_empty()).then(|| (outcome.chunk_id.clone(), names))
        })
        .collect()
}

/// The targets grouped by the questions they ask again, in the order each set
/// first appears, so each group is one batch of one question set.
fn escalation_groups(targets: &[(String, Vec<String>)]) -> IndexMap<Vec<String>, Vec<String>> {
    let mut groups: IndexMap<Vec<String>, Vec<String>> = IndexMap::new();
    for (chunk_id, names) in targets {
        groups
            .entry(names.clone())
            .or_default()
            .push(chunk_id.clone());
    }
    groups
}

/// One question asked again: the second model's answer, written into `slot`
/// in place of the first model's, or the first model's kept with the reason
/// the second model gave none (`too_large` when its chunk was never sent).
fn escalated_answer(
    name: &str,
    slot: &mut Decision,
    again: &ChunkOutcome,
    label: &str,
    first_label: &str,
    too_large: bool,
) -> EscalatedAnswer {
    let (first_key, first_p) = (answer_key(slot), top_p(slot));
    let replacement = again
        .decisions
        .iter()
        .flatten()
        .find(|d| d.question == name)
        .cloned();
    match replacement {
        Some(replacement) => {
            let key = answer_key(&replacement);
            let p = top_p(&replacement);
            *slot = replacement;
            EscalatedAnswer {
                question: name.to_string(),
                label: key,
                p,
                by: label.to_string(),
                first_label: first_key,
                first_p,
                error: None,
                too_large: false,
            }
        }
        None => EscalatedAnswer {
            question: name.to_string(),
            label: first_key.clone(),
            p: first_p,
            by: first_label.to_string(),
            first_label: first_key,
            first_p,
            error: Some(
                again
                    .error
                    .clone()
                    .unwrap_or_else(|| "no answer".to_string()),
            ),
            too_large,
        },
    }
}

/// The first model's outcomes with each answer the second model gave in its
/// place, and what the escalation did. An answer the second model failed to
/// give, or was never asked because its chunk was too large for it, keeps the
/// first model's. Pure, so the merge is testable without a model.
fn merge_escalation(
    questions: &IndexMap<String, Question>,
    first: Vec<ChunkOutcome>,
    first_label: &str,
    run: EscalationRun,
) -> (Vec<ChunkOutcome>, EscalationSummary) {
    let label = {
        let short = model_label(&run.model);
        if short == first_label {
            decision_model_display_label(&run.model)
        } else {
            short
        }
    };
    let first_decided = first.iter().filter(|o| o.decisions.is_some()).count();
    let by_chunk: std::collections::HashMap<&str, &ChunkOutcome> = run
        .outcomes
        .iter()
        .map(|outcome| (outcome.chunk_id.as_str(), outcome))
        .collect();
    let mut items = Vec::new();
    let merged = first
        .into_iter()
        .map(|mut outcome| {
            let Some(again) = by_chunk.get(outcome.chunk_id.as_str()) else {
                return outcome;
            };
            let Some(decisions) = outcome.decisions.as_mut() else {
                return outcome;
            };
            let mut item_answers = Vec::new();
            for name in questions.keys() {
                let Some(slot) = decisions.iter_mut().find(|d| &d.question == name) else {
                    continue;
                };
                // Asked again only when unsure; the second model's outcome
                // holds exactly the questions asked again.
                let asked = again.error.is_some()
                    || again
                        .decisions
                        .iter()
                        .flatten()
                        .any(|d| &d.question == name);
                if !asked || !unsure(slot, run.below) {
                    continue;
                }
                let too_large = run.too_large.contains(&outcome.chunk_id);
                item_answers.push(escalated_answer(
                    name,
                    slot,
                    again,
                    &label,
                    first_label,
                    too_large,
                ));
            }
            if !item_answers.is_empty() {
                items.push(EscalatedItem {
                    chunk_id: outcome.chunk_id.clone(),
                    answers: item_answers,
                });
            }
            outcome
        })
        .collect();
    let all_answers = || items.iter().flat_map(|item: &EscalatedItem| &item.answers);
    let answers = all_answers().count();
    let decided = all_answers().filter(|a| a.error.is_none()).count();
    let changed = all_answers()
        .filter(|a| a.error.is_none() && a.label != a.first_label)
        .count();
    let errors = answers - decided;
    let too_large_ids = items
        .iter()
        .filter(|item| item.answers.iter().any(|answer| answer.too_large))
        .map(|item| item.chunk_id.clone())
        .collect();
    // A chunk too large for the second model did not fail there: it was
    // never sent, and the result says so on its own line.
    let failed: Vec<ChunkOutcome> = run
        .outcomes
        .iter()
        .filter(|outcome| !run.too_large.contains(&outcome.chunk_id))
        .cloned()
        .collect();
    let summary = EscalationSummary {
        model: run.model,
        label,
        tier: run.tier,
        first_label: first_label.to_string(),
        below: run.below,
        first_decided,
        escalated: items.len(),
        answers,
        decided,
        changed,
        errors,
        items,
        error_groups: error_groups(&failed),
        too_large_ids,
    };
    (merged, summary)
}

fn ids_line(ids: &[String], more: usize) -> String {
    let mut line = ids.join(", ");
    if more > 0 {
        line.push_str(&format!(" (and {more} more)"));
    }
    line
}

/// What a line says after an answer that was escalated: the first model's
/// answer, or that the chunk was too large for the second model, or that the
/// second model failed, and the first answer stands.
fn escalated_note(answer: &EscalatedAnswer, escalation: &EscalationSummary) -> String {
    let first = &escalation.first_label;
    match &answer.error {
        None => format!(
            " (escalated; {first} said {}, P {:.2})",
            answer.first_label, answer.first_p
        ),
        Some(_) if answer.too_large => format!(
            " (too large for {}; {first}'s answer stands)",
            escalation.label
        ),
        Some(_) => format!(" (escalation failed; {first}'s answer stands)"),
    }
}

/// One compact line per chunk, for small batches only.
fn chunk_line(
    questions: &IndexMap<String, Question>,
    outcome: &ChunkOutcome,
    escalation: Option<&EscalationSummary>,
) -> String {
    let Some(decisions) = &outcome.decisions else {
        return format!(
            "- {}: failed: {}",
            outcome.chunk_id,
            outcome.error.as_deref().unwrap_or("no answer")
        );
    };
    let escalated = escalation.and_then(|escalation| {
        escalation
            .items
            .iter()
            .find(|item| item.chunk_id == outcome.chunk_id)
            .map(|item| (item, escalation))
    });
    let parts: Vec<String> = questions
        .iter()
        .filter_map(|(name, question)| {
            let decision = decisions.iter().find(|d| &d.question == name)?;
            let note = escalated
                .and_then(|(item, escalation)| {
                    item.answers
                        .iter()
                        .find(|answer| &answer.question == name)
                        .map(|answer| escalated_note(answer, escalation))
                })
                .unwrap_or_default();
            // Never a bare "fail": an agent reads that as a no, and a near
            // miss below the threshold is not one.
            let verdict = match (decision.passed, in_review(question, decision)) {
                (Some(true), false) => " pass",
                (Some(true), true) => " pass, review",
                (Some(false), false) => " below threshold",
                (Some(false), true) => " below threshold, review",
                (None, true) => " review",
                (None, false) => "",
            };
            Some(match &decision.answer {
                Answer::Binary { .. } => {
                    format!("{name} P(yes) {:.2}{verdict}{note}", p_yes(decision))
                }
                _ => format!(
                    "{name} {} (P {:.2}){verdict}{note}",
                    answer_key(decision),
                    reported_p(decision)
                ),
            })
        })
        .collect();
    format!("- {}: {}", outcome.chunk_id, parts.join("; "))
}

/// One question's paragraph of the summary text. `list_answers` adds the
/// chunk ids of each answer of a choice or score, for a batch too big for a
/// line per chunk, so every chunk's label can still be reported.
fn question_paragraph(question: &QuestionSummary, chunks: usize, list_answers: bool) -> String {
    let rule = match (&question.pass_on, question.threshold) {
        (Some(on), Some(p)) => format!(", passes at P({on}) >= {p:.2}"),
        _ => String::new(),
    };
    let mut facts: Vec<String> = Vec::new();
    if let Some(groups) = &question.groups {
        // G-3: four groups that do not overlap, each with its bounds, so the
        // counts add up to the chunks answered. A group whose range is empty
        // under this threshold (a pass at 0.9 is always confident) is left out;
        // its count is zero by construction.
        let on = question.pass_on.as_deref().unwrap_or("yes");
        let no_until = groups.pass_from.max(groups.review_from);
        let mut parts = vec![format!(
            "yes, confident {} (P({on}) {:.2} and up)",
            groups.yes_confident, groups.confident_from
        )];
        if groups.pass_from < groups.confident_from {
            parts.push(format!(
                "yes, to review {} ({:.2} up to {:.2})",
                groups.yes_review, groups.pass_from, groups.confident_from
            ));
        }
        if groups.review_from < no_until {
            parts.push(format!(
                "no, to review {} ({:.2} up to {no_until:.2})",
                groups.no_review, groups.review_from
            ));
        }
        parts.push(format!(
            "no, confident {} (under {:.2})",
            groups.no_confident,
            groups.review_from.min(groups.pass_from)
        ));
        facts.push(parts.join("; "));
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
    if !question.review_ids.is_empty() {
        text.push_str(&format!(
            "\n  review: {}",
            ids_line(&question.review_ids, question.more_review)
        ));
    }
    if let (true, Some(ids), Some(counts)) =
        (list_answers, &question.answer_ids, &question.answer_counts)
    {
        for (answer, listed) in ids.iter().filter(|(_, listed)| !listed.is_empty()) {
            let total = counts.get(answer).copied().unwrap_or(listed.len());
            text.push_str(&format!(
                "\n  {answer}: {}",
                ids_line(listed, total.saturating_sub(listed.len()))
            ));
        }
    }
    text
}

/// The escalation's paragraph: how many chunks each model decided, what the
/// second model changed, the chunks too large for it, and its failures.
fn escalation_paragraph(escalation: &EscalationSummary) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let tier = escalation
        .tier
        .map(|tier| format!(" ({})", tier_word(tier)))
        .unwrap_or_default();
    let answers = if escalation.answers == escalation.escalated {
        String::new()
    } else {
        format!(
            " ({} answer{})",
            escalation.answers,
            plural(escalation.answers)
        )
    };
    let mut text = format!(
        "\nEscalation: {} decided by {}, {} escalated to {}{tier}{answers}, where a top answer \
         was under {:.2}.",
        escalation.first_decided,
        escalation.first_label,
        escalation.escalated,
        escalation.label,
        escalation.below,
    );
    if escalation.answers == 0 {
        text.push_str(&format!(
            " No answer was under {:.2}, so {} was asked nothing.",
            escalation.below, escalation.label
        ));
        return text;
    }
    let too_large = &escalation.too_large_ids;
    let too_large_answers = escalation
        .items
        .iter()
        .flat_map(|item| &item.answers)
        .filter(|answer| answer.too_large)
        .count();
    // The answers the second model was asked about: every escalated answer
    // but those of a chunk too large for it.
    let sent = escalation.answers.saturating_sub(too_large_answers);
    if sent > 0 {
        text.push_str(&format!(
            " {} gave {} of them, {} different from {}'s. Every answer below is final: {}'s \
             where it answered, {}'s elsewhere.",
            escalation.label,
            escalation.decided,
            escalation.changed,
            escalation.first_label,
            escalation.label,
            escalation.first_label,
        ));
    }
    if !too_large.is_empty() {
        let one = too_large.len() == 1;
        let outcome = if sent == 0 {
            format!(
                "so {} was asked nothing and every answer below is {}'s",
                escalation.label, escalation.first_label
            )
        } else {
            format!(
                "so {} not sent to it and {}'s answer stands for {}",
                if one { "it was" } else { "they were" },
                escalation.first_label,
                if one { "it" } else { "them" },
            )
        };
        text.push_str(&format!(
            " {} escalated chunk{} {} too large for {}'s window beside the questions asked \
             again, {outcome}.",
            too_large.len(),
            plural(too_large.len()),
            if one { "was" } else { "were" },
            escalation.label,
        ));
    }
    let failed = escalation.errors.saturating_sub(too_large_answers);
    if failed > 0 {
        text.push_str(&format!(
            " {failed} escalated answer{} failed, so {}'s answer stands for {}.",
            plural(failed),
            escalation.first_label,
            if failed == 1 { "it" } else { "them" },
        ));
    }
    text.push_str(&escalation_detail_lines(escalation));
    text
}

/// The lines under the escalation paragraph that name the chunks the second
/// model never answered: those too large for its window, then each group of
/// chunks whose escalation failed.
fn escalation_detail_lines(escalation: &EscalationSummary) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let too_large = &escalation.too_large_ids;
    let mut text = String::new();
    if !too_large.is_empty() {
        let (listed, more) = capped(too_large.clone(), LISTED_ERROR_IDS);
        let how_large = escalation
            .items
            .iter()
            .flat_map(|item| &item.answers)
            .find(|answer| answer.too_large)
            .and_then(|answer| answer.error.as_deref())
            .map(|why| format!(" {why}"))
            .unwrap_or_default();
        text.push_str(&format!(
            "\nToo large for {} ({} chunk{}): {}.{how_large}",
            escalation.label,
            too_large.len(),
            plural(too_large.len()),
            ids_line(&listed, more),
        ));
    }
    for group in &escalation.error_groups {
        let hit = group.chunk_ids.len() + group.more;
        text.push_str(&format!(
            "\nEscalation failed ({hit} chunk{}): {}: {}",
            plural(hit),
            ids_line(&group.chunk_ids, group.more),
            group.message
        ));
    }
    text
}

/// For a batch too big for a line per chunk: one line per chunk asked again,
/// the final answer beside the first model's, at most [`PER_CHUNK_LINES`].
fn escalated_lines(escalation: &EscalationSummary) -> String {
    if escalation.items.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "\nEscalated (final answer, then {}'s):",
        escalation.first_label
    );
    for item in escalation.items.iter().take(PER_CHUNK_LINES) {
        let answers: Vec<String> = item
            .answers
            .iter()
            .map(|answer| match &answer.error {
                None => format!(
                    "{} {} P {:.2}, was {} P {:.2}",
                    answer.question, answer.label, answer.p, answer.first_label, answer.first_p
                ),
                Some(_) if answer.too_large => format!(
                    "{} {} P {:.2}, too large for {}",
                    answer.question, answer.label, answer.p, escalation.label
                ),
                Some(_) => format!(
                    "{} {} P {:.2}, escalation failed",
                    answer.question, answer.label, answer.p
                ),
            })
            .collect();
        text.push_str(&format!("\n- {}: {}", item.chunk_id, answers.join("; ")));
    }
    let rest = escalation.items.get(PER_CHUNK_LINES..).unwrap_or_default();
    if !rest.is_empty() {
        // Never capped, so every chunk's model can be told: named here, the
        // escalation model's answer (or, where it failed or the chunk was too
        // large for it, the first model's); not named, the first model's.
        let (too_large, rest): (Vec<&EscalatedItem>, Vec<&EscalatedItem>) = rest
            .iter()
            .partition(|item| item.answers.iter().any(|answer| answer.too_large));
        let (failed, answered): (Vec<&EscalatedItem>, Vec<&EscalatedItem>) = rest
            .into_iter()
            .partition(|item| item.answers.iter().any(|answer| answer.error.is_some()));
        let rest_len = too_large.len() + failed.len() + answered.len();
        let ids = |items: &[&EscalatedItem]| {
            items
                .iter()
                .map(|item| item.chunk_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        text.push_str(&format!(
            "\n(and {rest_len} more escalated chunk{}, their final answers in the lists above.",
            if rest_len == 1 { "" } else { "s" }
        ));
        if !answered.is_empty() {
            text.push_str(&format!(
                " Answered by {}: {}.",
                escalation.label,
                ids(&answered)
            ));
        }
        if !too_large.is_empty() {
            text.push_str(&format!(
                " Too large for {}, so {}'s answer stands: {}.",
                escalation.label,
                escalation.first_label,
                ids(&too_large)
            ));
        }
        if !failed.is_empty() {
            text.push_str(&format!(
                " Failed, so {}'s answer stands: {}.",
                escalation.first_label,
                ids(&failed)
            ));
        }
        text.push(')');
    }
    text
}

/// The low-confidence line: every chunk with a top answer under the bound,
/// all of them, and what to do with them.
fn low_confidence_line(summary: &DecideSummary) -> String {
    let below = summary.low_confidence_below;
    let ids = &summary.low_confidence_ids;
    let escalated = summary.escalation.is_some();
    if ids.is_empty() {
        return if escalated {
            format!("\nAfter escalation no top answer is under {below:.2}.")
        } else {
            format!("\nLow confidence (a top answer under {below:.2}): none.")
        };
    }
    let count = format!("{} of {} chunks", ids.len(), summary.decided);
    if escalated {
        let them = if ids.len() == 1 { "it" } else { "them" };
        format!(
            "\nStill low confidence after escalation (a top answer under {below:.2}), {count}: \
             {}. Read {them} before you report {them}, or report {them} as unresolved.",
            ids.join(", ")
        )
    } else {
        format!(
            "\nLow confidence (a top answer under {below:.2}), {count}: {}. To ask another model \
             about exactly these, repeat this call with `only_ids` set to them and that `model`, \
             or use `escalate` to do both in one call.",
            ids.join(", ")
        )
    }
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
            // By its bare name, a Baseten model's alias, which `model` also
            // takes, so the agent has no provider slug to repeat (R3-F8).
            decision_model_short_name(model),
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
        "decide: {} chunk{} x {} question{}{model} in {:.1} s: {} decided, {} failed.",
        summary.chunks,
        plural(summary.chunks),
        questions.len(),
        plural(questions.len()),
        summary.elapsed_ms as f64 / 1000.0,
        summary.decided,
        summary.errors,
    );
    let per_chunk =
        outcomes.len() <= PER_CHUNK_LINES && outcomes.len() * questions.len() <= PER_CHUNK_CELLS;
    if let Some(escalation) = &summary.escalation {
        text.push_str(&escalation_paragraph(escalation));
    }
    for question in &summary.questions {
        text.push_str(&question_paragraph(question, summary.chunks, !per_chunk));
    }
    if let (Some(escalation), false) = (&summary.escalation, per_chunk) {
        text.push_str(&escalated_lines(escalation));
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
    if per_chunk {
        text.push_str("\nPer chunk:");
        for outcome in outcomes {
            text.push('\n');
            text.push_str(&chunk_line(questions, outcome, summary.escalation.as_ref()));
        }
    }
    if summary.decided > 0 {
        text.push_str(&low_confidence_line(summary));
    }
    if summary.calibrated == Some(false) {
        text.push_str(
            "\nThese probabilities are uncalibrated (the model wrote them, or most of its \
             probability fell outside the offered answers): treat them as rough.",
        );
    }
    // R1-8: the batch id stays in the structured result, where the desktop
    // links the call's history; no tool takes it, and a model that reads it
    // repeats it to the person.
    text.push_str(
        "\nEvery call, with the exact text it read, is in this chat's decision history. A chunk \
         listed under review is unresolved, not a no: read it before you report it, or report \
         it as unresolved. A decision model can also be confidently wrong, so check a few \
         passes.",
    );
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

    /// Resolve a `model` argument against what `decision_models` lists (see
    /// [`resolve_model_name`]).
    async fn named_model(
        &self,
        session: &DecisionSession,
        wanted: &str,
    ) -> Result<DecisionModelRef, String> {
        resolve_model_name(self.service, session, wanted).await
    }

    /// The model a `decide` call asks first: the one its `model` argument
    /// names, else the chat's own default, checked as a named model is, so a
    /// default this chat may not use is refused with its reason up front.
    /// `None` leaves the choice to the service.
    async fn model_to_ask(
        &self,
        session: &DecisionSession,
        wanted: Option<&str>,
    ) -> Result<Option<DecisionModelRef>, String> {
        match wanted.map(str::trim) {
            Some(wanted) if !wanted.is_empty() => {
                Ok(Some(self.named_model(session, wanted).await?))
            }
            _ => match &session.chat_default_model {
                Ok(Some(default)) => resolve_named(self.service, session, &default.label())
                    .await
                    .map(Some)
                    .map_err(|problem| problem.for_default(default)),
                Ok(None) => Ok(None),
                Err(why) => Err(format!(
                    "{why}, so this chat's default decision model is unknown. Name a decision \
                     model as this call's model."
                )),
            },
        }
    }

    /// The first model's name and tier, from its answers, else from the
    /// listing (nothing was decided, so no answer names it).
    async fn first_model_of(
        &self,
        session: &DecisionSession,
        model: Option<&DecisionModelRef>,
        outcomes: &[ChunkOutcome],
    ) -> Option<(DecisionModelRef, ProviderTier)> {
        let answered = outcomes
            .iter()
            .find_map(|o| o.decisions.as_ref().and_then(|d| d.first()))
            .map(|d| (d.model.clone(), d.tier));
        if answered.is_some() {
            return answered;
        }
        self.service
            .model_entry_for(Some(session), model)
            .await
            .map(|entry| (entry.model_ref(), entry.tier))
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
        let wanted_models = vote_models(&params)?;
        let escalation = escalation_ask(&params)?;

        let batch_id = uuid::Uuid::now_v7().to_string();
        let session = self
            .session(session_id, enforced)
            .await?
            .with_origin(DecisionOrigin::Tool, Some(batch_id.clone()))
            .with_origin_label(Some(DECIDE_ORIGIN_LABEL.to_string()))
            .with_turn(crate::decision::turns::current_turn(session_id));
        if let Some(wanted) = wanted_models {
            return self
                .handle_vote(&session, &wanted, &questions, chunks, &batch_id, cancel)
                .await;
        }
        let model = self.model_to_ask(&session, params.model.as_deref()).await?;

        // The escalation model is checked before the first model is asked
        // anything: a model this chat may not use refuses the whole call with
        // nothing sent. Its window is not: only the chunks the first model is
        // unsure of are sent to it, so a chunk too large for it is known, and
        // left out with its first answer standing, once the first model has
        // answered.
        let escalation = match escalation {
            Some(ask) => Some(
                self.check_escalation(&session, model.as_ref(), &ask, &questions)
                    .await?,
            ),
            None => None,
        };
        let low_below = escalation
            .as_ref()
            .map_or(LOW_CONFIDENCE, |(_, ask)| ask.below);

        let started = Instant::now();
        // The chunks the escalation may ask again, each at its place in the call.
        let chunk_texts: std::collections::HashMap<String, (usize, DecisionChunk)> =
            match &escalation {
                Some(_) => chunks
                    .iter()
                    .map(|(place, chunk)| (chunk.id.clone(), (*place, chunk.clone())))
                    .collect(),
                None => std::collections::HashMap::new(),
            };
        let run = self.service.decide_batch_at(
            Some(&session),
            model.as_ref(),
            &questions,
            chunks,
            &batch_id,
        );
        let outcomes = tokio::select! {
            _ = cancel.cancelled() => return Err(CANCELLED.to_string()),
            result = run => result.map_err(|e| e.to_string())?,
        };
        let first_model = self
            .first_model_of(&session, model.as_ref(), &outcomes)
            .await;
        let (outcomes, escalated) = match escalation {
            Some((escalation_model, ask)) => {
                let first_label = first_model
                    .as_ref()
                    .map(|(model, _)| model_label(model))
                    .unwrap_or_else(|| "the first model".to_string());
                let run = self
                    .run_escalation(
                        &session,
                        escalation_model,
                        ask.below,
                        &questions,
                        &outcomes,
                        &chunk_texts,
                        &batch_id,
                        cancel,
                    )
                    .await?;
                let (merged, summary) = merge_escalation(&questions, outcomes, &first_label, run);
                (merged, Some(summary))
            }
            None => (outcomes, None),
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut summary = summarize(&questions, &outcomes, &batch_id, elapsed_ms, low_below);
        if let Some((first, tier)) = first_model {
            summary.model = Some(first);
            summary.tier = Some(tier);
        }
        summary.escalation = escalated;
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

    /// `escalate`'s model, resolved and checked the way any decide model is
    /// before anything is sent: named as `model` takes it (a model this chat
    /// may not use is refused with its reason), then every model-wide check a
    /// decision makes before it sends, and the question kinds. A refusal
    /// refuses the call before the first model is asked anything. The size
    /// check is not made here: which chunks, and which of their questions,
    /// are asked again is known only once the first model has answered, so
    /// [`Self::run_escalation`] checks each escalated chunk against the
    /// second model's window then, and leaves out only the ones too large
    /// for it, whose first answers stand.
    async fn check_escalation(
        &self,
        session: &DecisionSession,
        first: Option<&DecisionModelRef>,
        ask: &EscalationAsk,
        questions: &IndexMap<String, Question>,
    ) -> Result<(DecisionModelRef, EscalationAsk), String> {
        let model = resolve_named(self.service, session, &ask.model)
            .await
            .map_err(|problem| match problem {
                NameProblem::Refused(why) => format!(
                    "Nothing was decided: {}, the model to escalate to, can't be used in this \
                     chat. {why}",
                    name_as_written(&ask.model)
                ),
                other => format!("Nothing was decided. {}", other.for_name(&ask.model)),
            })?;
        let refused = |why: String| {
            format!(
                "Nothing was decided: {}, the model to escalate to, can't answer this call. {why}",
                model_in_words(&model)
            )
        };
        let first = match first {
            Some(first) => Some(first.clone()),
            None => self
                .service
                .model_entry_for(Some(session), None)
                .await
                .map(|entry| entry.model_ref()),
        };
        if first.as_ref() == Some(&model) {
            return Err(format!(
                "Nothing was decided: the model to escalate to is {}, the model that decides \
                 first. Name a different model to escalate to, or another first model.",
                model_in_words(&model)
            ));
        }
        let source = ModelSource::of(Some(session), Some(&model));
        self.service
            .check_model_takes(session, &model, questions, source)
            .await
            .map_err(|error| refused(error.to_string()))?;
        Ok((model, ask.clone()))
    }

    /// The escalation stage: each chunk whose top answer to a question is
    /// less probable than `below` is asked those questions again of `model`,
    /// one batch per set of questions, under the call's batch id, so the
    /// decision history holds both stages, each row naming its model. Before
    /// a batch is sent, each of its chunks is checked against `model`'s
    /// window beside that batch's questions: a chunk too large for it is left
    /// out of the batch, never sent, and its first answers stand. A batch
    /// the service refuses as a whole fails its chunks, whose first answers
    /// then stand.
    #[allow(clippy::too_many_arguments)]
    async fn run_escalation(
        &self,
        session: &DecisionSession,
        model: DecisionModelRef,
        below: f64,
        questions: &IndexMap<String, Question>,
        first: &[ChunkOutcome],
        chunk_texts: &std::collections::HashMap<String, (usize, DecisionChunk)>,
        batch_id: &str,
        cancel: &CancellationToken,
    ) -> Result<EscalationRun, String> {
        let tier = self
            .service
            .model_entry_for(Some(session), Some(&model))
            .await
            .map(|entry| entry.tier);
        let targets = escalation_targets(questions, first, below);
        let mut answered: std::collections::HashMap<String, ChunkOutcome> =
            std::collections::HashMap::new();
        let mut too_large = std::collections::HashSet::new();
        for (names, ids) in escalation_groups(&targets) {
            let asked: IndexMap<String, Question> = names
                .iter()
                .filter_map(|name| Some((name.clone(), questions.get(name)?.clone())))
                .collect();
            let texts = |ids: &[String]| -> CallChunks {
                ids.iter()
                    .filter_map(|id| chunk_texts.get(id).cloned())
                    .collect()
            };
            // Only what fits the second model's window is sent to it. When
            // the model cannot be opened to tell, the whole batch goes to the
            // service, whose own checks refuse it and record the refusal.
            let split = self
                .service
                .split_by_window(&model, &asked, texts(&ids))
                .await;
            let (ids, chunks) = match split {
                Ok(split) => {
                    for (id, error) in split.too_large {
                        too_large.insert(id.clone());
                        answered.insert(
                            id.clone(),
                            ChunkOutcome {
                                chunk_id: id,
                                decisions: None,
                                error: Some(error.to_string()),
                                billed_usage: None,
                            },
                        );
                    }
                    let ids: Vec<String> = split
                        .fits
                        .iter()
                        .map(|(_, chunk)| chunk.id.clone())
                        .collect();
                    (ids, split.fits)
                }
                Err(_) => {
                    let chunks = texts(&ids);
                    (ids, chunks)
                }
            };
            if chunks.is_empty() {
                continue;
            }
            let run =
                self.service
                    .decide_batch_at(Some(session), Some(&model), &asked, chunks, batch_id);
            let result = tokio::select! {
                _ = cancel.cancelled() => return Err(CANCELLED.to_string()),
                result = run => result,
            };
            match result {
                Ok(outcomes) => {
                    for outcome in outcomes {
                        answered.insert(outcome.chunk_id.clone(), outcome);
                    }
                }
                Err(error) => {
                    for id in ids {
                        answered.insert(
                            id.clone(),
                            ChunkOutcome {
                                chunk_id: id,
                                decisions: None,
                                error: Some(error.to_string()),
                                billed_usage: None,
                            },
                        );
                    }
                }
            }
        }
        // In call order, as the first stage's outcomes are.
        let outcomes = targets
            .iter()
            .filter_map(|(id, _)| answered.remove(id))
            .collect();
        Ok(EscalationRun {
            model,
            tier,
            below,
            outcomes,
            too_large,
        })
    }

    /// G-1: every chunk asked of each of 2 to 5 models, one after another so a
    /// local model is not asked to serve several at once. Each model's run is
    /// an ordinary batch under this call's batch id, so the decision log has
    /// one row per model per chunk.
    async fn handle_vote(
        &self,
        session: &DecisionSession,
        wanted: &[String],
        questions: &IndexMap<String, Question>,
        chunks: CallChunks,
        batch_id: &str,
        cancel: &CancellationToken,
    ) -> Result<CallToolResult, String> {
        let mut models: Vec<DecisionModelRef> = Vec::with_capacity(wanted.len());
        for name in wanted {
            let model = self.named_model(session, name).await?;
            if models.contains(&model) {
                return Err(format!(
                    "The vote names {} twice. Name each model once.",
                    model_in_words(&model)
                ));
            }
            models.push(model);
        }
        let chunk_ids: Vec<String> = chunks.iter().map(|(_, chunk)| chunk.id.clone()).collect();
        let started = Instant::now();
        let mut runs = Vec::with_capacity(models.len());
        for model in models {
            let tier = self
                .service
                .model_entry_for(Some(session), Some(&model))
                .await
                .map(|entry| entry.tier);
            let run = self.service.decide_batch_at(
                Some(session),
                Some(&model),
                questions,
                chunks.clone(),
                batch_id,
            );
            let outcomes = tokio::select! {
                _ = cancel.cancelled() => return Err(CANCELLED.to_string()),
                result = run => result.map_err(|e| e.to_string()),
            };
            runs.push(vote::ModelRun {
                model,
                tier,
                outcomes,
            });
        }
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let summary = vote::summarize_vote(questions, &chunk_ids, &runs, batch_id, elapsed_ms);
        let text = vote::render_vote(&summary, questions, &chunk_ids, &runs);
        let decided_any = summary.models.iter().any(|model| model.decided > 0);
        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: serde_json::to_value(&summary).ok(),
            is_error: Some(!decided_any),
            meta: None,
        })
    }

    async fn handle_decision_models(
        &self,
        session_id: &str,
        enforced: bool,
    ) -> Result<CallToolResult, String> {
        let session = self.session(session_id, enforced).await?;
        // The service's listing gives each model the reason its own gates
        // would refuse it with, a Crew grant included, so this tool,
        // `GET /decisions/models` and the Monitors dialog say the same thing.
        let listed = self.service.models_for(Some(&session)).await;
        // The model a `decide` call that names none would use in this chat,
        // by the service's own order, so the agent sizes chunks for it.
        let default = self
            .service
            .model_entry_for(Some(&session), None)
            .await
            .map(|entry| entry.model_ref());
        let rows: Vec<ModelRow> = listed
            .into_iter()
            .map(|(entry, reason)| ModelRow {
                default: default.as_ref() == Some(&entry.model_ref()),
                ..ModelRow::from_entry(&entry, reason)
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

/// Resolve a decision model's name against what `decision_models` lists for
/// `session`: as provider/model, or by a model's bare name or alias
/// (`joey-1`, `tev1:0.8b`) when that names exactly one listed model. A model
/// this chat may not use is refused with its reason. `decide`'s `model` and
/// `models`, and a phase gate's `model` (G-8), all resolve here, so a name
/// that works in one works in the others.
///
/// The refusal is one plain sentence that a person reads on the decide card
/// and the agent reads as its result: each model by its name and where it
/// runs (`tev1:0.8b on this computer`, `joey-1 on Baseten`), never by
/// `provider/model`, except where only that spelling tells two models apart.
pub(crate) async fn resolve_model_name(
    service: &DecisionService,
    session: &DecisionSession,
    wanted: &str,
) -> Result<DecisionModelRef, String> {
    resolve_named(service, session, wanted)
        .await
        .map_err(|problem| problem.for_name(wanted))
}

/// Why a decision model's name did not resolve for a chat.
#[derive(Debug, Clone, PartialEq)]
enum NameProblem {
    /// No listed model answers to it. Holds each model this chat may use,
    /// in words.
    NotSetUp { usable: Vec<String> },
    /// Several listed models answer to it: each one in words (where it runs
    /// only for a model this chat may use, its bare name otherwise), and the
    /// provider/model spelling that names it alone.
    Ambiguous(Vec<(String, String)>),
    /// It names a listed model this chat may not use, for this reason.
    Refused(String),
}

impl NameProblem {
    /// For a name the call wrote: `decide`'s `model` or `escalate`, a vote's
    /// `models`, a gate's model.
    fn for_name(&self, wanted: &str) -> String {
        let name = name_as_written(wanted);
        match self {
            Self::NotSetUp { usable } => format!(
                "A decision model named {name} is not set up. {}",
                usable_sentence(usable)
            ),
            Self::Ambiguous(models) => format!(
                "{name} names more than one decision model: {}. Name one of them by \
                 provider/model: {}.",
                and_list(
                    &models
                        .iter()
                        .map(|(words, _)| words.clone())
                        .collect::<Vec<_>>()
                ),
                or_list(
                    &models
                        .iter()
                        .map(|(_, spelling)| spelling.clone())
                        .collect::<Vec<_>>()
                )
            ),
            Self::Refused(reason) => reason.clone(),
        }
    }

    /// For the chat's own default decision model, which no call named.
    fn for_default(&self, default: &DecisionModelRef) -> String {
        let named = model_in_words(default);
        match self {
            Self::NotSetUp { usable } => format!(
                "This chat's default decision model, {named}, is not set up. {} Name one of \
                 them as this call's model, or choose another default in the chat's Monitors \
                 dialog.",
                usable_sentence(usable)
            ),
            Self::Ambiguous(_) => format!(
                "This chat's default decision model, {named}, can't be used here. {}",
                self.for_name(&default.label())
            ),
            Self::Refused(reason) => {
                format!("This chat's default decision model, {named}, can't be used here. {reason}")
            }
        }
    }
}

/// [`resolve_model_name`], with the reason a name did not resolve kept apart.
async fn resolve_named(
    service: &DecisionService,
    session: &DecisionSession,
    wanted: &str,
) -> Result<DecisionModelRef, NameProblem> {
    let listed = service.models_for(Some(session)).await;
    let found = match find_named(&listed, wanted) {
        Ok(found) => found,
        Err(several) => {
            // Where a model runs is said only for one this chat may use, as the
            // NotSetUp list says it: a refused model's place can name a server
            // (`the Ollama server at 10.0.0.5`) the chat has no business learning.
            return Err(NameProblem::Ambiguous(
                several
                    .iter()
                    .map(|(entry, reason)| {
                        let model = entry.model_ref();
                        (
                            if reason.is_none() {
                                entry.in_words()
                            } else {
                                decision_model_short_name(&model)
                            },
                            decision_model_display_label(&model),
                        )
                    })
                    .collect(),
            ));
        }
    };
    let Some((entry, reason)) = found else {
        return Err(NameProblem::NotSetUp {
            usable: listed
                .iter()
                .filter(|(_, reason)| reason.is_none())
                .map(|(entry, _)| entry.in_words())
                .collect(),
        });
    };
    if let Some(reason) = reason {
        return Err(NameProblem::Refused(reason.clone()));
    }
    Ok(entry.model_ref())
}

/// The name a call wrote, as a sentence says it: the model's own name when it
/// was written as provider/model (`test_decision/gpt-giant` is `gpt-giant`),
/// else the name as written.
fn name_as_written(wanted: &str) -> String {
    let wanted = wanted.trim();
    match DecisionModelRef::parse(wanted) {
        Some(parsed) => decision_model_short_name(&parsed),
        None => wanted.to_string(),
    }
}

/// The models a chat may use, as the sentence after a refusal says them.
fn usable_sentence(usable: &[String]) -> String {
    if usable.is_empty() {
        "No decision model this chat may use is set up.".to_string()
    } else {
        format!("This chat may use {}.", or_list(usable))
    }
}

fn and_list(items: &[String]) -> String {
    joined(items, "and")
}

fn or_list(items: &[String]) -> String {
    joined(items, "or")
}

fn joined(items: &[String], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., tail] => format!("{} {last} {tail}", init.join(", ")),
    }
}

/// The listed model `wanted` names, for a surface that resolves a name
/// before it has a chat (`biorouter decide --model`, R3-F4): by
/// provider/model first, else by a bare model id or alias that exactly one
/// listed model answers to, the rule [`resolve_model_name`] applies for the
/// agent's `decide` and a phase gate. `Ok(None)` when nothing listed answers
/// to it; `Err` holds the provider/model spellings of every listed model a
/// bare name matched, when it matched more than one. Whether the model may
/// read a chat is left to the service, which refuses it with its reason.
pub fn listed_model_named(
    listed: &[(DecisionModelEntry, Option<String>)],
    wanted: &str,
) -> Result<Option<DecisionModelRef>, Vec<String>> {
    find_named(listed, wanted)
        .map(|hit| hit.map(|(entry, _)| entry.model_ref()))
        .map_err(|several| {
            several
                .iter()
                .map(|(entry, _)| decision_model_display_label(&entry.model_ref()))
                .collect()
        })
}

/// `models`, when the call names them: 2 to 5, and never with `model`.
fn vote_models(params: &DecideParams) -> Result<Option<Vec<String>>, String> {
    let Some(models) = &params.models else {
        return Ok(None);
    };
    if params
        .model
        .as_deref()
        .is_some_and(|model| !model.trim().is_empty())
    {
        return Err(
            "Name one model in `model`, or 2 to 5 in `models` for a vote, not both.".to_string(),
        );
    }
    let models: Vec<String> = models
        .iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect();
    if !(vote::MIN_VOTE_MODELS..=vote::MAX_VOTE_MODELS).contains(&models.len()) {
        return Err(format!(
            "A vote takes {} to {} models in `models`, not {}. For one model use `model`.",
            vote::MIN_VOTE_MODELS,
            vote::MAX_VOTE_MODELS,
            models.len()
        ));
    }
    Ok(Some(models))
}

/// A model as the service lists it for a chat: the entry, and the reason this
/// chat may not use it (`None` when it may).
type ListedModel = (DecisionModelEntry, Option<String>);

/// The listed model `wanted` names: by provider/model first, else by a bare
/// model id or alias that exactly one listed model answers to. `Err` holds
/// every model a bare name matched, each with its refusal reason, when it
/// matched more than one.
fn find_named<'a>(
    listed: &'a [ListedModel],
    wanted: &str,
) -> Result<Option<&'a ListedModel>, Vec<&'a ListedModel>> {
    let wanted = wanted.trim();
    if let Some(parsed) = DecisionModelRef::parse(wanted) {
        if let Some(hit) = listed.iter().find(|(entry, _)| entry.answers_to(&parsed)) {
            return Ok(Some(hit));
        }
    }
    let bare: Vec<&ListedModel> = listed
        .iter()
        .filter(|(entry, _)| entry.model == wanted || entry.display_name == wanted)
        .collect();
    match bare.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        many => Err(many.to_vec()),
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
    /// on how much probability lands on the offered answers). Calibrated means
    /// the numbers are the model's own distribution, not that they have been
    /// validated on the caller's data.
    pub calibrated: Option<bool>,
    pub kinds: Vec<String>,
    pub context_tokens: usize,
    /// How much text one chunk can carry on this model, by the size check's
    /// own arithmetic: what to size chunks by, rather than the window.
    pub usable: UsableChunk,
    /// A body size the server refuses above (Ollama's System One: 64 KiB).
    pub max_request_bytes: Option<usize>,
    /// The most options a choice question may offer it.
    pub max_choice_options: usize,
    /// Whether `decide` uses this model in this chat when `model` is left out.
    pub default: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
}

impl ModelRow {
    fn from_entry(entry: &DecisionModelEntry, reason: Option<String>) -> Self {
        // Only Ollama's System One limits the body (`system_one_caps`).
        let max_request_bytes = (entry.provider == OLLAMA_DECISION_PROVIDER
            && entry.adapter == DecisionAdapter::SystemOne)
            .then_some(OLLAMA_MAX_REQUEST_BYTES);
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
            usable: usable_chunk(entry.context_tokens, max_request_bytes),
            max_request_bytes,
            max_choice_options: choice_option_limit(&entry.model_ref()),
            default: false,
            available: reason.is_none(),
            unavailable_reason: reason,
        }
    }
}

/// How `decision_models` names a model (R3-F8): by its alias or bare name
/// (`joey-1`, `tev1:0.8b`), which `decide`'s `model` takes, so the agent has
/// no provider slug to repeat to the person. Only a name that more than one
/// listed model answers to is given as provider/model, the spelling `model`
/// then needs.
fn listed_name(row: &ModelRow, rows: &[ModelRow]) -> String {
    let short = &row.display_name;
    let shared = rows
        .iter()
        .filter(|other| &other.display_name == short || &other.model == short)
        .count();
    if shared > 1 || short.trim().is_empty() {
        row.name.clone()
    } else {
        short.clone()
    }
}

/// The models as the model reads them: those this chat may use first.
pub fn render_models(rows: &[ModelRow], private_chat: bool) -> String {
    if rows.is_empty() {
        // A download is the person's choice, and no tool installs one.
        return format!(
            "No decision model is available. Ask the user to install one: {}",
            crate::decision::INSTALL_ADVICE
        );
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
        let name = listed_name(row, rows);
        let options = if row.max_choice_options < MAX_CHOICE_OPTIONS
            && row.kinds.iter().any(|kind| kind == "choice")
        {
            format!(" (a choice of at most {} options)", row.max_choice_options)
        } else {
            String::new()
        };
        let bytes = row
            .max_request_bytes
            .map(|bytes| format!(", and a request of at most {bytes} bytes"))
            .unwrap_or_default();
        text.push_str(&format!(
            "\n- {name}: {}{}, {}; takes {}{options}; window {} tokens, of which a chunk can \
             fill about {} (about {} characters of prose, or {} of numbers, tables, ids or code, \
             less the questions' own length{bytes}).",
            tier_word(row.tier),
            if row.local {
                ", runs on this computer"
            } else {
                ""
            },
            adapter_words(row.adapter),
            row.kinds.join(", "),
            row.context_tokens,
            row.usable.tokens,
            row.usable.prose_chars,
            row.usable.number_chars,
        ));
        if row.default {
            text.push_str(" decide uses this model here when `model` is left out.");
        }
        if let Some(reason) = &row.unavailable_reason {
            text.push_str(&format!(" Not available to this chat: {reason}"));
        }
    }
    if available.is_empty() {
        text.push_str("\nNone of these may read this chat, so decide will be refused here.");
    } else {
        let default = rows.iter().find(|row| row.default && row.available);
        text.push_str(&match default {
            Some(row) => format!(
                "\nName one in decide's `model` as listed here; left out, decide uses {} here, \
                 so size chunks for it.",
                listed_name(row, rows)
            ),
            None => "\nName one in decide's `model` as listed here, or leave `model` out for this \
                     chat's default decision model, else the configured default, else the first \
                     model this chat may use."
                .to_string(),
        });
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
        // The sentence alone: the result is marked as an error, and the decide
        // card already says the call failed above it, so an "Error:" in front
        // only repeated that.
        Ok(result.unwrap_or_else(|error| CallToolResult::error(vec![Content::text(error)])))
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
    /// state that contains `[state-p=0.95]` (the model's `text_mark`) gets
    /// that P(yes) for every question, a question whose instructions contain
    /// `[p=0.42]` gets P(yes) = 0.42 (or that much on its first option), and
    /// every other question `default_p`.
    #[derive(Clone)]
    pub(crate) struct FakeModel {
        pub(crate) entry: DecisionModelEntry,
        pub(crate) default_p: f64,
        pub(crate) calls: Arc<AtomicUsize>,
        pub(crate) fail_all: bool,
        /// The mark this model reads its P(yes) for a chunk from, so two
        /// models can answer the same chunk differently (`[esc-p=` for a
        /// second model, beside the first's `[state-p=`).
        pub(crate) text_mark: &'static str,
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
            let state_p = marked_p(&request.context.render_text(), self.text_mark);
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
            load_refusal: None,
        }
    }

    pub(crate) fn model(entry: DecisionModelEntry, default_p: f64) -> FakeModel {
        FakeModel {
            entry,
            default_p,
            calls: Arc::new(AtomicUsize::new(0)),
            fail_all: false,
            text_mark: "[state-p=",
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

    /// [`service`] that hands every decision log entry to `sink`.
    pub(crate) fn service_with_sink(
        models: Vec<FakeModel>,
        sink: Arc<dyn crate::decision::service::DecisionSink>,
    ) -> &'static DecisionService {
        Box::leak(Box::new(
            DecisionService::new(Arc::new(FakeSource { models }))
                .with_use_gate(Arc::new(Allow))
                .with_sink(sink),
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
            "spell out abbreviations",
            "pass_option yes",
            "lists under review",
            "the review list catches the rest",
            "add escalate {\"model\": ",
            "for escalate too",
        ] {
            assert!(DECIDE_DESCRIPTION.contains(needle), "missing {needle:?}");
        }
        // The escalate bullet sits with the other rules, before the example.
        assert!(
            DECIDE_DESCRIPTION.find("add escalate") < DECIDE_DESCRIPTION.find("Example, 600"),
            "{DECIDE_DESCRIPTION}"
        );
        // The fixed 0.4 to 0.6 band is gone, and the description no longer
        // teaches a bare question at 0.9, which passed 1 of 5 true notes.
        assert!(!DECIDE_DESCRIPTION.contains("0.4 to 0.6"));
        assert!(!DECIDE_DESCRIPTION.contains("\"threshold\": 0.9"));
    }

    /// The first JSON value in `text` after `marker`, read the way decide
    /// reads its arguments.
    fn json_after(text: &str, marker: &str) -> serde_json::Value {
        let (_, rest) = text
            .split_once(marker)
            .unwrap_or_else(|| panic!("no {marker:?}"));
        serde_json::Deserializer::from_str(rest)
            .into_iter::<serde_json::Value>()
            .next()
            .unwrap_or_else(|| panic!("no JSON after {marker:?}"))
            .unwrap_or_else(|e| panic!("the example after {marker:?} is not JSON: {e}"))
    }

    /// The agent copies the description's example, so it must be a call
    /// decide takes, in the choice form that was measured to keep true cases.
    #[test]
    fn the_description_example_is_a_valid_call() {
        let questions = json_after(DECIDE_DESCRIPTION, "neurology notes: questions ");
        let chunks = json_after(DECIDE_DESCRIPTION, "over chunks ");
        let params: DecideParams =
            serde_json::from_value(json!({"questions": questions, "chunks": chunks}))
                .expect("the example fits decide's schema");
        let (built, chunks) = build_batch(&params).expect("the example is a valid call");
        assert_eq!(chunks.len(), 1);
        let has_ms = &built["has_ms"];
        assert!(
            has_ms.instructions.contains("RRMS, SPMS or PPMS"),
            "the abbreviations are in the question itself"
        );
        let DecisionKind::Choice { options } = &has_ms.kind else {
            panic!("has_ms is the choice form: {has_ms:?}")
        };
        assert_eq!(options.keys().collect::<Vec<_>>(), ["yes", "no"]);
        assert!(options.values().all(|meaning| meaning.is_some()));
        assert_eq!(
            has_ms.threshold,
            Some(Threshold::Option {
                name: "yes".into(),
                p: 0.5
            })
        );
        assert_eq!(built["edss_stated"].kind, DecisionKind::Binary {});
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

    /// The text a person and the model read names a Baseten model by its
    /// alias, which `model` also takes; the structured result keeps the
    /// slug the decision log records.
    #[test]
    fn the_decide_text_names_a_baseten_model_by_its_alias() {
        let slug = "decision-models/baseten-joey-2b-latest";
        let summary = DecideSummary {
            kind: "decide_batch".into(),
            batch_id: "batch-1".into(),
            chunks: 1,
            decided: 1,
            errors: 0,
            model: Some(DecisionModelRef::new("baseten_decision", slug)),
            tier: Some(ProviderTier::Public),
            calibrated: Some(true),
            elapsed_ms: 10,
            questions: Vec::new(),
            error_groups: Vec::new(),
            low_confidence_below: LOW_CONFIDENCE,
            low_confidence_ids: Vec::new(),
            escalation: None,
        };
        let text = render_summary(&summary, &IndexMap::new(), &[]);
        assert!(text.contains(" with joey-1 (public)"), "{text}");
        assert!(!text.contains("decision-models/"), "{text}");
        let unknown = DecideSummary {
            model: Some(DecisionModelRef::new(
                "baseten_decision",
                "decision-models/baseten-newer-4b-latest",
            )),
            ..summary.clone()
        };
        let text = render_summary(&unknown, &IndexMap::new(), &[]);
        assert!(
            text.contains(" with baseten-newer-4b-latest (public)"),
            "an unknown slug loses only its prefix: {text}"
        );
        assert_eq!(summary.model.map(|m| m.model).as_deref(), Some(slug));
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
        // The one list that is never capped is the low-confidence ids, which
        // an agent decides again by id (`only_ids`): here every chunk is
        // unsure on edss_stated, so it names each once. Every other list is
        // capped, so a chunk past the caps appears there and nowhere else.
        let low = body
            .lines()
            .find(|line| line.starts_with("Low confidence"))
            .unwrap_or_else(|| panic!("no low-confidence line: {body}"));
        assert!(low.contains("196 of 196 chunks"), "{low}");
        assert_eq!(
            body.matches("note-0100").count(),
            1,
            "a big batch must not list every chunk outside the low-confidence ids: {body}"
        );
        assert!(low.contains("note-0100"), "{low}");
        assert!(!body.contains("Per chunk"), "{body}");
        assert!(body.contains("196 decided, 4 failed"), "{body}");
        assert!(body.contains("has_ms"), "{body}");
        assert!(
            body.contains("yes, confident 196 (P(yes) 0.90 and up); no, to review 0"),
            "has_ms at 0.95 is a clear pass: {body}"
        );
        assert!(body.contains("(and 156 more)"), "{body}");
        assert!(
            body.contains("no, to review 196 (0.20 up to 0.80)"),
            "edss at 0.5 under a 0.8 threshold is to review: {body}"
        );
        assert_eq!(
            body.matches("no, confident").count(),
            2,
            "a choice with no threshold has no groups: {body}"
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
        assert!(
            !body.contains(&summary.batch_id)
                && !body.contains("Batch")
                && !body.contains("batch "),
            "R1-8: the batch id stays out of the text the model reads: {body}"
        );
        assert_eq!(summary.chunks, 200);
        assert_eq!(summary.errors, 4);
        assert_eq!(summary.error_groups.len(), 1);
        assert_eq!(summary.error_groups[0].chunk_ids.len(), 4);
        let has_ms = &summary.questions[0];
        assert_eq!(has_ms.passed, Some(196));
        assert_eq!(has_ms.passing_ids.len(), LISTED_IDS);
        assert_eq!(has_ms.more_passing, 196 - LISTED_IDS);
        assert_eq!(has_ms.review, Some(0));
        let edss = &summary.questions[1];
        assert_eq!(edss.review, Some(196));
        assert_eq!(edss.review_ids.len(), LISTED_IDS);
        assert_eq!(edss.more_review, 196 - LISTED_IDS);
        assert_eq!(summary.questions[2].review, None);
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
        assert!(
            body.contains("- a: done P(yes) 0.20 below threshold, review"),
            "{body}"
        );
        assert!(body.contains("- b: failed:"), "{body}");
        assert!(!says_bare_fail(&body), "{body}");
    }

    /// A per-chunk verdict printed as the bare word "fail", which an agent
    /// reads as a no.
    fn says_bare_fail(body: &str) -> bool {
        body.lines()
            .any(|line| line.ends_with(" fail") || line.contains(" fail;"))
    }

    /// One decide call with one question `has_ms` over chunks named `ids`, on
    /// a fake model that gives every question `p`.
    async fn decide_with(
        p: f64,
        question: serde_json::Value,
        ids: &[&str],
    ) -> (String, DecideSummary) {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            p,
        );
        let (client, session) = client_with(vec![local], false).await;
        let chunks: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| json!({"id": id, "text": format!("note {id}")}))
            .collect();
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"has_ms": question}, "chunks": chunks}),
            enforced(),
        )
        .await;
        assert_eq!(result.is_error, Some(false), "{}", text(&result));
        let summary = serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        (text(&result), summary)
    }

    /// The review band, as a table: from 0.2 up to the larger of the threshold
    /// and 0.8.
    #[test]
    fn needs_review_table() {
        for (p, threshold, expected) in [
            (0.19, None, false),
            (0.20, None, true),
            (0.79, None, true),
            (0.80, None, false),
            (0.36, Some(0.9), true),
            (0.85, Some(0.9), true),
            (0.90, Some(0.9), false),
            (0.85, Some(0.5), false),
            (0.60, Some(0.5), true),
            (0.05, Some(0.5), false),
        ] {
            assert_eq!(
                needs_review(p, threshold),
                expected,
                "needs_review({p}, {threshold:?})"
            );
        }
    }

    /// Measured: a true RRMS note scored 0.36 against a 0.9 threshold. It was
    /// printed "fail", the band 0.4 to 0.6 missed it, and the agent reported
    /// it as a no. It is now listed for review and never called a fail.
    #[tokio::test]
    async fn a_near_miss_below_the_threshold_is_listed_for_review() {
        let (body, summary) = decide_with(
            0.36,
            json!({"instructions": "Does this note state MS?", "threshold": 0.9}),
            &["a", "b", "c"],
        )
        .await;
        assert!(
            body.contains("no, to review 3 (0.20 up to 0.90); no, confident 0 (under 0.20)"),
            "{body}"
        );
        assert!(body.contains("\n  review: a, b, c"), "{body}");
        assert!(
            body.contains("- a: has_ms P(yes) 0.36 below threshold, review"),
            "{body}"
        );
        assert!(!says_bare_fail(&body), "{body}");
        assert!(
            body.contains("A chunk listed under review is unresolved, not a no"),
            "{body}"
        );
        assert!(body.contains("check a few passes"), "{body}");
        assert_eq!(summary.questions[0].review, Some(3));
        assert_eq!(summary.questions[0].review_ids, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn a_clear_no_is_not_listed_for_review() {
        let (body, summary) = decide_with(
            0.05,
            json!({"instructions": "Does this note state MS?", "threshold": 0.9}),
            &["a", "b"],
        )
        .await;
        assert!(!body.contains("\n  review:"), "{body}");
        assert!(body.contains("no, to review 0"), "{body}");
        assert!(body.contains("no, confident 2 (under 0.20)"), "{body}");
        assert!(
            body.contains("- a: has_ms P(yes) 0.05 below threshold\n"),
            "{body}"
        );
        assert_eq!(summary.questions[0].review, Some(0));
        assert!(summary.questions[0].review_ids.is_empty());
    }

    /// A pass under 0.8 is not a clear pass, and a yes_no with no threshold is
    /// reviewed on P(yes) itself.
    #[tokio::test]
    async fn a_pass_short_of_a_clear_pass_is_listed_for_review() {
        let (body, _) = decide_with(
            0.6,
            json!({"instructions": "Does this note state MS?", "threshold": 0.5}),
            &["a"],
        )
        .await;
        assert!(
            body.contains("- a: has_ms P(yes) 0.60 pass, review"),
            "{body}"
        );
        assert!(body.contains("\n  review: a"), "{body}");

        let (body, summary) = decide_with(
            0.45,
            json!({"instructions": "Does this note state MS?"}),
            &["a"],
        )
        .await;
        assert!(body.contains("- a: has_ms P(yes) 0.45 review"), "{body}");
        assert!(body.contains("no, to review 1 (0.20 up to 0.50)"), "{body}");
        assert_eq!(summary.questions[0].review, Some(1));

        let (body, _) = decide_with(
            0.85,
            json!({"instructions": "Does this note state MS?", "threshold": 0.5}),
            &["a"],
        )
        .await;
        assert!(body.contains("- a: has_ms P(yes) 0.85 pass\n"), "{body}");
        assert!(!body.contains("\n  review:"), "{body}");
    }

    /// The choice form the description teaches is reviewed on P(pass_option);
    /// a choice with no threshold has no pass, so nothing to review against.
    #[tokio::test]
    async fn a_choice_with_a_threshold_is_reviewed_on_its_pass_option() {
        let choice = |threshold: Option<f64>| {
            let mut q = json!({
                "type": "choice",
                "instructions": "Does this note state the patient has multiple sclerosis?",
                "options": {"yes": "The patient has MS.", "no": "The patient does not have MS."},
            });
            if let Some(t) = threshold {
                q["threshold"] = json!(t);
                q["pass_option"] = json!("yes");
            }
            q
        };
        let (body, summary) = decide_with(0.36, choice(Some(0.5)), &["a", "b"]).await;
        assert_eq!(summary.questions[0].review, Some(2), "{body}");
        assert!(body.contains("no, to review 2 (0.20 up to 0.50)"), "{body}");
        assert!(body.contains("\n  review: a, b"), "{body}");
        assert!(body.contains("(P 0.36) below threshold, review"), "{body}");

        let (body, summary) = decide_with(0.36, choice(None), &["a"]).await;
        assert_eq!(summary.questions[0].review, None, "{body}");
        assert!(!body.contains("to review"), "{body}");
        assert!(!body.contains("\n  review:"), "{body}");
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
        // The model by its name, never `test_decision/gone`, and the ones this chat
        // may use by their names and where they run.
        assert_eq!(
            text(&missing),
            "This chat's default decision model, gone, is not set up. This chat may use tev1 \
             on this computer or cloud. Name one of them as this call's model, or choose \
             another default in the chat's Monitors dialog."
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
        // Plain words for the person on the card and the agent alike: the name
        // as written, never in backticks, and the models it may use by where
        // they run, never `provider/model`.
        assert_eq!(
            body,
            "A decision model named gpt-giant is not set up. This chat may use tev1 on this \
             computer."
        );
        let unknown_bare = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": [{"id": "a", "text": "x"}], "model": "gpt-giant"}),
            enforced(),
        )
        .await;
        assert_eq!(unknown_bare.is_error, Some(true));
        assert_eq!(text(&unknown_bare), body);
        assert!(
            !body.contains('`') && !body.contains("test_decision"),
            "{body}"
        );
    }

    /// B-2: a bare model name or alias names the one listed model it answers
    /// to, so `joey-1` works as `baseten_decision/joey-1` does; a name two
    /// listed models answer to is refused with both spellings.
    #[tokio::test]
    async fn a_bare_name_that_names_one_listed_model_is_that_model() {
        let mut alias = entry(
            "decision-models/test-joey-2b-latest",
            ProviderTier::Private,
            true,
            DecisionAdapter::SystemOne,
        );
        alias.display_name = "joey-1".into();
        let mut twin = entry(
            "twin",
            ProviderTier::Private,
            true,
            DecisionAdapter::SystemOne,
        );
        twin.provider = "other_decision".into();
        let (client, session) = client_with(
            vec![
                model(alias, 0.9),
                model(
                    entry(
                        "twin",
                        ProviderTier::Private,
                        true,
                        DecisionAdapter::SystemOne,
                    ),
                    0.9,
                ),
                model(twin, 0.9),
            ],
            false,
        )
        .await;
        let ask = |name: &str| {
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": [{"id": "a", "text": "x"}], "model": name})
        };
        for name in ["joey-1", "decision-models/test-joey-2b-latest"] {
            let result = call(&client, &session, "decide", ask(name), enforced()).await;
            assert_eq!(result.is_error, Some(false), "{name}: {}", text(&result));
            let summary: DecideSummary =
                serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
            assert_eq!(
                summary.model.unwrap().model,
                "decision-models/test-joey-2b-latest"
            );
        }
        let both = call(&client, &session, "decide", ask("twin"), enforced()).await;
        assert_eq!(both.is_error, Some(true));
        let body = text(&both);
        assert!(
            body.contains("names more than one decision model")
                && body.contains("test_decision/twin")
                && body.contains("other_decision/twin"),
            "{body}"
        );
        // provider/model still picks one of the two.
        let one = call(
            &client,
            &session,
            "decide",
            ask("other_decision/twin"),
            enforced(),
        )
        .await;
        assert_eq!(one.is_error, Some(false), "{}", text(&one));
    }

    /// A name two listed models answer to says where each runs only for a model
    /// this chat may use: a refused model is named alone, so the refusal never
    /// tells the chat where a model it may not reach is served.
    #[tokio::test]
    async fn an_ambiguous_name_says_where_only_the_usable_models_run() {
        let usable = entry(
            "twin",
            ProviderTier::Private,
            true,
            DecisionAdapter::SystemOne,
        );
        let mut refused = entry(
            "twin",
            ProviderTier::Public,
            false,
            DecisionAdapter::SystemOne,
        );
        refused.provider = "baseten_decision".into();
        // A private chat may not use a public model.
        let (client, session) =
            client_with(vec![model(usable, 0.9), model(refused, 0.9)], true).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": [{"id": "a", "text": "x"}], "model": "twin"}),
            enforced(),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let body = text(&result);
        assert!(
            body.starts_with("twin names more than one decision model: ")
                && body.contains("twin on this computer")
                && !body.contains("on Baseten")
                && body.contains("test_decision/twin")
                && body.contains("baseten_decision/twin"),
            "{body}"
        );
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
        // R3-F8: a model is listed by the name `model` takes, no provider slug.
        assert!(!body.contains("test_decision/"), "{body}");
        let tev1 = body.find("\n- tev1: ").unwrap();
        let cloud = body.find("\n- cloud: ").unwrap();
        assert!(tev1 < cloud, "usable models come first: {body}");
        assert!(body.contains("Not available to this chat"), "{body}");
        // The listing says what the desktop and the CLI say: calibrated is
        // the model's own distribution, not a check against the user's data.
        assert!(
            body.contains("System One, the model's own probabilities, not validated on your data"),
            "{body}"
        );
        let rows: Vec<ModelRow> =
            serde_json::from_value(result.structured_content.unwrap()["models"].clone()).unwrap();
        let cloud = rows.iter().find(|r| r.model == "cloud").unwrap();
        assert!(!cloud.available && cloud.unavailable_reason.is_some());
        let tev1 = rows.iter().find(|r| r.model == "tev1").unwrap();
        assert!(tev1.available && tev1.local);
        assert_eq!(tev1.kinds, ["yes_no", "choice", "score"]);
        // W1/W3: the size to chunk by, in tokens and characters, and which
        // model a decide that names none uses in this chat.
        let usable = crate::decision::service::usable_chunk(4_096, None);
        assert_eq!(tev1.usable, usable);
        assert_eq!(
            usable.tokens,
            ((4_096 - 128) as f64 / 1.15).floor() as usize
        );
        assert_eq!(usable.prose_chars, usable.tokens * 4);
        assert_eq!(usable.number_chars, usable.tokens);
        assert!(tev1.default && !cloud.default);
        assert!(
            body.contains(&format!(
                "window 4096 tokens, of which a chunk can fill about {} (about {} characters of \
                 prose, or {} of numbers",
                usable.tokens, usable.prose_chars, usable.number_chars
            )),
            "{body}"
        );
        assert!(body.contains("left out, decide uses tev1 here"), "{body}");
    }

    /// R3-F8: a Baseten model is listed by its alias, never its slug, and a
    /// bare name two providers share keeps provider/model, the spelling
    /// `model` then needs.
    #[test]
    fn decision_models_lists_aliases_and_keeps_a_slug_only_when_it_is_needed() {
        let joey = DecisionModelEntry {
            provider: "baseten_decision".into(),
            model: "decision-models/baseten-joey-2b-latest".into(),
            display_name: "joey-1".into(),
            ..entry("x", ProviderTier::Public, false, DecisionAdapter::SystemOne)
        };
        let local = entry(
            "tev1:0.8b",
            ProviderTier::Private,
            true,
            DecisionAdapter::SystemOne,
        );
        let twin = DecisionModelEntry {
            provider: "other_decision".into(),
            ..local.clone()
        };
        let rows = |entries: Vec<DecisionModelEntry>| -> Vec<ModelRow> {
            entries
                .iter()
                .map(|entry| ModelRow::from_entry(entry, None))
                .collect()
        };
        let text = render_models(&rows(vec![joey.clone(), local.clone()]), false);
        assert!(text.contains("\n- joey-1: public"), "{text}");
        assert!(text.contains("\n- tev1:0.8b: private"), "{text}");
        assert!(!text.contains("decision-models/"), "{text}");
        assert!(!text.contains("baseten_decision"), "{text}");
        let text = render_models(&rows(vec![joey, local, twin]), false);
        assert!(text.contains("\n- test_decision/tev1:0.8b: "), "{text}");
        assert!(text.contains("\n- other_decision/tev1:0.8b: "), "{text}");
        assert!(text.contains("\n- joey-1: "), "{text}");
    }

    /// W3: a batch in which every chunk was refused still names the model it
    /// was sized for. F3: chunks refused for the same reason, each naming its
    /// own size, are one error group that counts them all.
    #[tokio::test]
    async fn an_all_refused_batch_names_its_model_and_groups_its_refusals() {
        let mut small = entry(
            "tiny",
            ProviderTier::Private,
            true,
            DecisionAdapter::SystemOne,
        );
        small.context_tokens = 512;
        let (client, session) = client_with(vec![model(small, 0.9)], false).await;
        let chunks: Vec<serde_json::Value> = (0..12)
            .map(|i| json!({"id": format!("n{i}"), "text": "word ".repeat(600 + 40 * i)}))
            .collect();
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}}, "chunks": chunks}),
            enforced(),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(summary.decided, 0);
        assert_eq!(summary.model.as_ref().unwrap().model, "tiny");
        assert_eq!(summary.error_groups.len(), 1, "{:?}", summary.error_groups);
        let group = &summary.error_groups[0];
        assert_eq!(group.chunk_ids.len() + group.more, 12);
        let body = text(&result);
        assert!(body.contains(" with tiny (private)"), "{body}");
        assert!(body.contains("Failed (12 chunks)"), "{body}");
    }

    #[test]
    fn errors_past_the_listed_kinds_are_counted_together() {
        let outcomes: Vec<ChunkOutcome> = (0..8)
            .map(|i| ChunkOutcome {
                chunk_id: format!("c{i}"),
                decisions: None,
                error: Some(format!(
                    "distinct failure kind {}",
                    ["a", "b", "c", "d", "e", "f", "g", "h"][i]
                )),
                billed_usage: None,
            })
            .collect();
        let groups = error_groups(&outcomes);
        assert_eq!(groups.len(), LISTED_ERRORS + 1);
        let last = groups.last().unwrap();
        assert_eq!(last.chunk_ids, ["c5", "c6", "c7"]);
        assert!(
            last.message.starts_with("3 other errors"),
            "{}",
            last.message
        );
        assert_eq!(
            error_shape("Chunk `n12` is about 2346 tokens", "n12"),
            "Chunk `…` is about # tokens"
        );
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
        let text = render_models(&[], false);
        assert!(
            text.starts_with("No decision model is available. Ask the user to install one: "),
            "{text}"
        );
        assert!(text.ends_with(crate::decision::INSTALL_ADVICE), "{text}");
        assert!(text.contains("biorouter models decision pull"), "{text}");
    }

    /// G-3: the four groups do not overlap and add up to the chunks answered,
    /// each says its bounds, and the review list is the two "to review" groups.
    #[tokio::test]
    async fn the_answer_groups_do_not_overlap_and_add_up() {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.5,
        );
        let (client, session) = client_with(vec![local], false).await;
        let chunks: Vec<serde_json::Value> = [0.95, 0.6, 0.3, 0.1, 0.02]
            .iter()
            .enumerate()
            .map(|(i, p)| json!({"id": format!("c{i}"), "text": format!("note [state-p={p}]")}))
            .collect();
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"has_ms": {"instructions": "MS?", "threshold": 0.5}},
                   "chunks": chunks}),
            enforced(),
        )
        .await;
        let body = text(&result);
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let groups = summary.questions[0].groups.unwrap();
        assert_eq!(
            (
                groups.yes_confident,
                groups.yes_review,
                groups.no_review,
                groups.no_confident
            ),
            (1, 1, 1, 2)
        );
        assert_eq!(
            groups.yes_confident + groups.yes_review + groups.no_review + groups.no_confident,
            summary.questions[0].decided
        );
        assert!(
            body.contains(
                "yes, confident 1 (P(yes) 0.80 and up); yes, to review 1 (0.50 up to 0.80); \
                 no, to review 1 (0.20 up to 0.50); no, confident 2 (under 0.20)"
            ),
            "{body}"
        );
        assert_eq!(summary.questions[0].review_ids, ["c1", "c2"]);
        assert_eq!(summary.questions[0].review, Some(2));

        // Under a threshold below 0.2, a weak pass is a yes to review, not a
        // confident one, and still adds up.
        let chunks = json!([{"id": "weak", "text": "note [state-p=0.15]"},
                            {"id": "none", "text": "note [state-p=0.05]"}]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "MS?", "threshold": 0.1}},
                   "chunks": chunks}),
            enforced(),
        )
        .await;
        let body = text(&result);
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let groups = summary.questions[0].groups.unwrap();
        assert_eq!(
            (
                groups.yes_confident,
                groups.yes_review,
                groups.no_review,
                groups.no_confident
            ),
            (0, 1, 0, 1),
            "{body}"
        );
        assert!(
            !body.contains("no, to review"),
            "an empty range is left out: {body}"
        );
        assert!(body.contains("no, confident 1 (under 0.10)"), "{body}");
    }

    /// Each logged row's (model, chunk) and batch id, and its (model, chunk
    /// number) as the decision history numbers it.
    #[derive(Default)]
    struct Rows(
        std::sync::Mutex<Vec<(String, String)>>,
        std::sync::Mutex<Vec<String>>,
        std::sync::Mutex<Vec<(String, Option<usize>)>>,
    );

    #[async_trait]
    impl crate::decision::service::DecisionSink for Rows {
        async fn record(
            &self,
            entry: crate::decision::service::DecisionLogEntry,
        ) -> anyhow::Result<()> {
            let model = entry.model.map(|m| m.model).unwrap_or_default();
            self.2
                .lock()
                .unwrap()
                .push((model.clone(), entry.chunk_index));
            self.0
                .lock()
                .unwrap()
                .push((model, entry.chunk_id.unwrap_or_default()));
            self.1
                .lock()
                .unwrap()
                .push(entry.batch_id.unwrap_or_default());
            Ok(())
        }
    }

    /// G-1: `models` asks every chunk of each model, logs one row per model per
    /// chunk under one batch id, and gives each model's answer and the majority,
    /// with a tie shown as a tie.
    #[tokio::test]
    async fn a_vote_asks_every_model_and_reports_the_majority() {
        let named = |name: &str, p: f64| {
            model(
                entry(
                    name,
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                p,
            )
        };
        let (a, b, c) = (
            named("alpha", 0.9),
            named("beta:1b", 0.9),
            named("gamma", 0.1),
        );
        let calls = [a.calls.clone(), b.calls.clone(), c.calls.clone()];
        let rows = Arc::new(Rows::default());
        let (client, session) = client_with(vec![], false).await;
        let client =
            client.with_service(test_support::service_with_sink(vec![a, b, c], rows.clone()));
        let chunks = json!([{"id": "n1", "text": "note one"}, {"id": "n2", "text": "note two"},
                            {"id": "n3", "text": "agreed [state-p=0.95]"}]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"has_ms": {"instructions": "MS?", "threshold": 0.5}},
                   "chunks": chunks,
                   "models": ["alpha", "beta:1b", "test_decision/gamma"]}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        for calls in &calls {
            assert_eq!(
                calls.load(Ordering::SeqCst),
                3,
                "every chunk of every model"
            );
        }
        let summary: VoteSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(summary.kind, "decide_vote");
        let question = &summary.questions[0];
        assert_eq!(question.majority.get("yes"), Some(&3));
        assert_eq!(question.ties, 0);
        assert_eq!((question.agreed, question.compared), (1, 3));
        assert_eq!(
            question.per_model[2].get("no"),
            Some(&2),
            "gamma said no twice"
        );
        assert!(
            body.contains("majority yes 3; tie 0; all models agreed on 1 of 3 chunks"),
            "{body}"
        );
        assert!(
            body.contains("- n1: has_ms yes (alpha yes, beta:1b yes, gamma no)"),
            "{body}"
        );
        let mut logged = rows.0.lock().unwrap().clone();
        logged.sort();
        let mut expected: Vec<(String, String)> = ["alpha", "beta:1b", "gamma"]
            .iter()
            .flat_map(|m| ["n1", "n2", "n3"].map(|c| (m.to_string(), c.to_string())))
            .collect();
        expected.sort();
        assert_eq!(logged, expected, "one row per model per chunk");
        assert!(
            rows.1
                .lock()
                .unwrap()
                .iter()
                .all(|b| *b == summary.batch_id),
            "every row under the call's one batch id"
        );

        // Two models that disagree tie.
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"has_ms": {"instructions": "MS?"}},
                   "chunks": [{"id": "n1", "text": "note one"}],
                   "models": ["alpha", "gamma"]}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert!(body.contains("majority no answers; tie 1"), "{body}");
        assert!(
            body.contains("- n1: has_ms tie (yes 1, no 1) (alpha yes, gamma no)"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_vote_is_refused_when_it_names_too_few_models_the_same_twice_or_with_model() {
        let named = |name: &str| {
            model(
                entry(
                    name,
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                0.9,
            )
        };
        let (client, session) = client_with(vec![named("alpha"), named("beta")], false).await;
        let ask = |extra: serde_json::Value| {
            let mut args = json!({"questions": {"q": {"instructions": "yes?"}},
                                  "chunks": [{"id": "a", "text": "a note"}]});
            for (key, value) in extra.as_object().unwrap() {
                args[key] = value.clone();
            }
            args
        };
        for (extra, expected) in [
            (json!({"models": ["alpha"]}), "2 to 5 models"),
            (
                json!({"models": ["alpha", "beta"], "model": "alpha"}),
                "not both",
            ),
            (json!({"models": ["alpha", "test_decision/alpha"]}), "twice"),
        ] {
            let result = call(&client, &session, "decide", ask(extra), enforced()).await;
            assert_eq!(result.is_error, Some(true));
            assert!(text(&result).contains(expected), "{}", text(&result));
        }
    }

    /// A `below` threshold is reported as its inner threshold's answer and
    /// bound; whether it passed is the decision's own `passed`.
    #[test]
    fn a_below_threshold_reports_its_inner_answer_and_bound() {
        let below = Threshold::Below(Box::new(Threshold::Option {
            name: "rrms".into(),
            p: 0.3,
        }));
        assert_eq!(pass_on(Some(&below)).as_deref(), Some("rrms"));
        assert_eq!(threshold_p(Some(&below)), Some(0.3));
        let low = Threshold::Below(Box::new(Threshold::AtLeastLevel { level: 2, p: 0.6 }));
        assert_eq!(pass_on(Some(&low)).as_deref(), Some("level >= 2"));
        assert_eq!(threshold_p(Some(&low)), Some(0.6));
    }

    /// The first model, `tev1`, reads a chunk's P(yes) from `[state-p=…]`; the
    /// second, `clef`, from `[esc-p=…]`, so each answers a chunk its own way.
    fn first_and_second(
        second_tier: ProviderTier,
        second_window: usize,
    ) -> (test_support::FakeModel, test_support::FakeModel) {
        let first = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.5,
        );
        let mut second_entry = entry("clef", second_tier, true, DecisionAdapter::SystemOne);
        second_entry.local = second_tier.is_private();
        second_entry.affiliation = second_entry
            .local
            .then_some(crate::privacy::affiliation::ModelAffiliation::Local);
        second_entry.context_tokens = second_window;
        let mut second = model(second_entry, 0.5);
        second.text_mark = "[esc-p=";
        (first, second)
    }

    /// The chunks `(id, first P(yes), second P(yes))` as decide takes them.
    fn marked_chunks(marks: &[(&str, f64, f64)]) -> Vec<serde_json::Value> {
        marks
            .iter()
            .map(|(id, first, second)| {
                json!({"id": id, "text": format!("note [state-p={first}] [esc-p={second}]")})
            })
            .collect()
    }

    /// The owner's QA ask: screen with tev1, send only the answers whose top
    /// probability is under 0.8 to a second model, and report the final
    /// labels. One call does it; both stages are in the decision history under
    /// the call's batch id, each row naming its model.
    #[tokio::test]
    async fn escalate_asks_the_second_model_about_only_the_unsure_answers() {
        let (first, second) = first_and_second(ProviderTier::Private, 4_096);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let rows = Arc::new(Rows::default());
        let (client, session) = client_with(vec![], false).await;
        let client = client.with_service(test_support::service_with_sink(
            vec![first, second],
            rows.clone(),
        ));
        let chunks = marked_chunks(&[
            ("c0", 0.95, 0.10), // confident yes: kept
            ("c1", 0.60, 0.10), // unsure yes: clef says no
            ("c2", 0.30, 0.20), // unsure no: clef agrees, now confident
            ("c3", 0.05, 0.90), // confident no: kept
            ("c4", 0.55, 0.50), // unsure yes: clef is unsure too
        ]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"has_ms": {"instructions": "MS?", "threshold": 0.5}},
                   "chunks": chunks, "model": "tev1",
                   "escalate": {"below": 0.8, "model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 5, "every chunk first");
        assert_eq!(second_calls.load(Ordering::SeqCst), 3, "only the unsure");

        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(summary.model.as_ref().unwrap().model, "tev1");
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!(escalation.label, "clef");
        assert_eq!(escalation.first_label, "tev1");
        assert_eq!(
            (
                escalation.first_decided,
                escalation.escalated,
                escalation.answers,
                escalation.decided,
                escalation.changed,
                escalation.errors
            ),
            (5, 3, 3, 3, 1, 0)
        );
        let ids: Vec<&str> = escalation
            .items
            .iter()
            .map(|i| i.chunk_id.as_str())
            .collect();
        assert_eq!(ids, ["c1", "c2", "c4"]);
        let c1 = &escalation.items[0].answers[0];
        assert_eq!(
            (c1.label.as_str(), c1.by.as_str(), c1.first_label.as_str()),
            ("no", "clef", "yes")
        );
        assert!((c1.p - 0.9).abs() < 1e-9 && (c1.first_p - 0.6).abs() < 1e-9);
        // The final answers are clef's where escalated and tev1's elsewhere:
        // c0 (tev1, 0.95) and c4 (clef, 0.50) pass.
        assert_eq!(summary.questions[0].passed, Some(2));
        assert_eq!(summary.questions[0].passing_ids, ["c0", "c4"]);
        assert_eq!(
            summary.low_confidence_ids,
            ["c4"],
            "still unsure after both"
        );

        assert!(
            body.contains(
                "Escalation: 5 decided by tev1, 3 escalated to clef (private), where a top \
                 answer was under 0.80. clef gave 3 of them, 1 different from tev1's."
            ),
            "{body}"
        );
        assert!(
            body.contains(
                "- c1: has_ms P(yes) 0.10 below threshold (escalated; tev1 said yes, P 0.60)"
            ),
            "{body}"
        );
        assert!(body.contains("- c0: has_ms P(yes) 0.95 pass\n"), "{body}");
        assert!(
            body.contains("Still low confidence after escalation (a top answer under 0.80), 1 of 5 chunks: c4."),
            "{body}"
        );

        let logged = rows.0.lock().unwrap().clone();
        let by = |m: &str| logged.iter().filter(|(model, _)| model == m).count();
        assert_eq!(
            (by("tev1"), by("clef")),
            (5, 3),
            "both stages, each by its model"
        );
        assert!(
            rows.1
                .lock()
                .unwrap()
                .iter()
                .all(|b| *b == summary.batch_id),
            "every row under the call's one batch id"
        );
        // The history numbers a chunk by its place in the call, so the second
        // model's rows name c1, c2 and c4 as Chunk 2, 3 and 5, never as the
        // first three chunks of a smaller batch.
        let mut numbered = rows.2.lock().unwrap().clone();
        numbered.sort();
        let places = |m: &str| -> Vec<Option<usize>> {
            numbered
                .iter()
                .filter(|(model, _)| model == m)
                .map(|(_, place)| *place)
                .collect()
        };
        assert_eq!(places("tev1"), [0, 1, 2, 3, 4].map(Some));
        assert_eq!(places("clef"), [1, 2, 4].map(Some));
    }

    #[tokio::test]
    async fn escalate_asks_nothing_when_every_answer_is_confident() {
        let (first, second) = first_and_second(ProviderTier::Private, 4_096);
        let second_calls = second.calls.clone();
        let (client, session) = client_with(vec![first, second], false).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": marked_chunks(&[("a", 0.95, 0.1), ("b", 0.02, 0.9)]),
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.unwrap();
        assert_eq!(
            (escalation.escalated, escalation.below),
            (0, LOW_CONFIDENCE)
        );
        assert!(escalation.items.is_empty());
        assert!(summary.low_confidence_ids.is_empty());
        assert!(
            body.contains("2 decided by tev1, 0 escalated to clef")
                && body.contains("No answer was under 0.80, so clef was asked nothing.")
                && body.contains("After escalation no top answer is under 0.80."),
            "{body}"
        );
    }

    /// Every chunk unsure, a big batch (no line per chunk), and a question
    /// asked again only where it was unsure: the escalated lines list the
    /// final answer beside the first, capped.
    #[tokio::test]
    async fn escalate_can_ask_again_about_every_chunk() {
        let (first, second) = first_and_second(ProviderTier::Private, 4_096);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let (client, session) = client_with(vec![first, second], false).await;
        let marks: Vec<(String, f64, f64)> =
            (0..50).map(|i| (format!("n{i:02}"), 0.6, 0.95)).collect();
        let marks: Vec<(&str, f64, f64)> = marks
            .iter()
            .map(|(id, a, b)| (id.as_str(), *a, *b))
            .collect();
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?", "threshold": 0.5},
                                 "also": {"instructions": "and this?"}},
                   "chunks": marked_chunks(&marks),
                   "model": "tev1", "escalate": {"below": 0.9, "model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 50);
        assert_eq!(second_calls.load(Ordering::SeqCst), 50);
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!((escalation.escalated, escalation.answers), (50, 100));
        assert_eq!(escalation.changed, 0, "yes stays yes");
        assert!(summary.low_confidence_ids.is_empty(), "{body}");
        assert_eq!(summary.questions[0].groups.unwrap().yes_confident, 50);
        assert!(
            body.contains("50 escalated to clef (private) (100 answers)"),
            "{body}"
        );
        assert!(
            body.contains("- n00: q yes P 0.95, was yes P 0.60; also yes P 0.95, was yes P 0.60"),
            "{body}"
        );
        let names: Vec<String> = (20..50).map(|i| format!("n{i:02}")).collect();
        assert!(
            body.contains(&format!(
                "(and 30 more escalated chunks, their final answers in the lists above. Answered \
                 by clef: {}.)",
                names.join(", ")
            )),
            "every escalated chunk is named, never capped: {body}"
        );
        assert!(!body.contains("Per chunk"), "{body}");
    }

    /// Gate H for the second model: a private chat refuses a public
    /// escalation model before the first model is asked anything.
    #[tokio::test]
    async fn an_escalation_model_the_chat_may_not_use_refuses_the_call_before_anything_is_sent() {
        let (first, second) = first_and_second(ProviderTier::Public, 4_096);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let (client, session) = client_with(vec![first, second], true).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": marked_chunks(&[("a", 0.6, 0.9)]),
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(true), "{body}");
        assert!(
            body.starts_with(
                "Nothing was decided: clef, the model to escalate to, can't be used in this chat."
            ),
            "{body}"
        );
        assert!(body.contains("private"), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 0, "stage 1 never ran");
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
    }

    /// The owner's case: the old preflight refused the whole call because
    /// the second model could hold no chunk, although it would never have
    /// been asked about any. The window is checked against what is
    /// escalated, so a call whose first answers are all confident runs, and
    /// the second model is asked nothing.
    #[tokio::test]
    async fn a_second_model_too_small_for_every_chunk_is_no_reason_to_refuse_confident_answers() {
        let (first, second) = first_and_second(ProviderTier::Private, 512);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let (client, session) = client_with(vec![first, second], false).await;
        let chunks = json!([
            {"id": "long-1", "text": format!("[state-p=0.97] {}", "word ".repeat(700))},
            {"id": "long-2", "text": format!("[state-p=0.03] {}", "word ".repeat(800))},
        ]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}}, "chunks": chunks,
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 2, "{body}");
        assert_eq!(second_calls.load(Ordering::SeqCst), 0, "{body}");
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(summary.decided, 2);
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!((escalation.escalated, escalation.errors), (0, 0));
        assert!(escalation.too_large_ids.is_empty());
        assert!(
            body.contains("No answer was under 0.80, so clef was asked nothing.")
                && !body.contains("too large")
                && !body.contains("Nothing was decided"),
            "{body}"
        );
    }

    /// Every chunk unsure and every chunk too large for the second model: the
    /// first model decides them all, nothing reaches the second model, and
    /// each line says why the first answer stands. Never an error, never a
    /// failure: nothing was sent that could fail.
    #[tokio::test]
    async fn unsure_chunks_too_large_for_the_second_model_keep_their_first_answers() {
        let (first, second) = first_and_second(ProviderTier::Private, 512);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let rows = Arc::new(Rows::default());
        let (client, session) = client_with(vec![], false).await;
        let client = client.with_service(test_support::service_with_sink(
            vec![first, second],
            rows.clone(),
        ));
        let chunks = json!([
            {"id": "long-1", "text": "word ".repeat(700)},
            {"id": "long-2", "text": "word ".repeat(800)},
        ]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}}, "chunks": chunks,
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 2, "the first pass ran");
        assert_eq!(second_calls.load(Ordering::SeqCst), 0, "nothing was sent");
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!((summary.decided, summary.errors), (2, 0));
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!(
            (
                escalation.escalated,
                escalation.answers,
                escalation.decided,
                escalation.errors
            ),
            (2, 2, 0, 2)
        );
        assert_eq!(escalation.too_large_ids, ["long-1", "long-2"]);
        assert!(escalation.error_groups.is_empty(), "nothing failed");
        for item in &escalation.items {
            let answer = &item.answers[0];
            assert_eq!(answer.by, "tev1", "{}", item.chunk_id);
            assert!(answer.too_large, "{}", item.chunk_id);
            assert!(
                answer
                    .error
                    .as_deref()
                    .is_some_and(|why| why.contains("its window is 512 tokens")),
                "{answer:?}"
            );
        }
        assert_eq!(summary.low_confidence_ids, ["long-1", "long-2"]);
        assert!(
            body.contains(
                "2 escalated chunks were too large for clef's window beside the questions asked \
                 again, so clef was asked nothing and every answer below is tev1's."
            ) && body.contains(
                "\nToo large for clef (2 chunks): long-1, long-2. Chunk `long-1` is about"
            ) && body.contains(
                "- long-1: q P(yes) 0.50 review (too large for clef; tev1's answer stands)"
            ) && !body.contains("gave 0 of them")
                && !body.contains("escalated answer")
                && !body.contains("Escalation failed"),
            "{body}"
        );
        // The second model has no row: it was asked nothing.
        let logged = rows.0.lock().unwrap().clone();
        assert!(
            logged.iter().all(|(model, _)| model == "tev1"),
            "{logged:?}"
        );

        let wire = result.structured_content.clone().unwrap();
        assert_eq!(
            wire["escalation"]["tooLargeIds"],
            json!(["long-1", "long-2"])
        );
        assert_eq!(
            wire["escalation"]["items"][0]["answers"][0]["tooLarge"],
            true
        );
    }

    /// A second model that can read some escalated chunks is asked about
    /// those. An unsure chunk too large for it is never sent to it: the first
    /// model's answer stands, and the result says so and why.
    #[tokio::test]
    async fn an_escalation_model_too_small_for_some_chunks_keeps_their_first_answers() {
        let (first, second) = first_and_second(ProviderTier::Private, 512);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let (client, session) = client_with(vec![first, second], false).await;
        let chunks = json!([
            {"id": "short", "text": "note [state-p=0.6] [esc-p=0.9]"},
            {"id": "long-1", "text": "word ".repeat(700)},
            {"id": "long-2", "text": "word ".repeat(800)},
        ]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}}, "chunks": chunks,
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 3, "{body}");
        assert_eq!(
            second_calls.load(Ordering::SeqCst),
            1,
            "only the chunk that fits reaches the second model: {body}"
        );
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!(
            (escalation.escalated, escalation.decided, escalation.errors),
            (3, 1, 2)
        );
        assert_eq!(escalation.too_large_ids, ["long-1", "long-2"]);
        let short = &escalation.items[0];
        assert_eq!(short.chunk_id, "short");
        assert_eq!(
            (
                short.answers[0].by.as_str(),
                short.answers[0].error.is_none(),
                short.answers[0].too_large
            ),
            ("clef", true, false)
        );
        for item in &escalation.items[1..] {
            let answer = &item.answers[0];
            assert_eq!(
                answer.by, "tev1",
                "the first answer stands for {}",
                item.chunk_id
            );
            assert!(answer.too_large && answer.error.is_some(), "{answer:?}");
        }
        assert!(
            escalation.error_groups.is_empty(),
            "a chunk too large is not a failure: {:?}",
            escalation.error_groups
        );
        assert_eq!(summary.low_confidence_ids, ["long-1", "long-2"]);
        assert!(
            body.contains(
                "clef gave 1 of them, 0 different from tev1's. Every answer below is final: \
                 clef's where it answered, tev1's elsewhere. 2 escalated chunks were too large \
                 for clef's window beside the questions asked again, so they were not sent to \
                 it and tev1's answer stands for them."
            ) && body.contains(
                "\nToo large for clef (2 chunks): long-1, long-2. Chunk `long-1` is about"
            ) && body.contains("- short: q P(yes) 0.90 (escalated; tev1 said yes, P 0.60)")
                && !body.contains("Escalation failed")
                && !body.contains("escalated answer"),
            "{body}"
        );
    }

    /// The second model failing on a chunk it was sent, and a chunk too
    /// large for it, are told apart: each keeps its first answer, the failure
    /// is grouped with its error, and the chunk too large is named on its own
    /// line.
    #[tokio::test]
    async fn a_failure_and_a_chunk_too_large_for_the_second_model_are_told_apart() {
        let (first, mut second) = first_and_second(ProviderTier::Private, 512);
        second.fail_all = true;
        let second_calls = second.calls.clone();
        let (client, session) = client_with(vec![first, second], false).await;
        let chunks = json!([
            {"id": "short", "text": "note [state-p=0.6]"},
            {"id": "long", "text": "word ".repeat(700)},
        ]);
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}}, "chunks": chunks,
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(
            second_calls.load(Ordering::SeqCst),
            1,
            "only `short` was sent"
        );
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!(
            (escalation.escalated, escalation.decided, escalation.errors),
            (2, 0, 2)
        );
        assert_eq!(escalation.too_large_ids, ["long"]);
        assert_eq!(escalation.error_groups.len(), 1);
        assert_eq!(escalation.error_groups[0].chunk_ids, ["short"]);
        assert_eq!(summary.low_confidence_ids, ["short", "long"]);
        assert!(
            body.contains(
                "1 escalated chunk was too large for clef's window beside the questions asked \
                 again, so it was not sent to it and tev1's answer stands for it. 1 escalated \
                 answer failed, so tev1's answer stands for it."
            ) && body.contains("\nToo large for clef (1 chunk): long. Chunk `long` is about")
                && body
                    .contains("\nEscalation failed (1 chunk): short: the decision server failed")
                && body.contains(
                    "- short: q P(yes) 0.60 review (escalation failed; tev1's answer stands)"
                )
                && body.contains(
                    "- long: q P(yes) 0.50 review (too large for clef; tev1's answer stands)"
                ),
            "{body}"
        );
    }

    /// The window is checked against the questions a chunk is asked again,
    /// not every question of the call: a chunk the second model could not
    /// read beside every question, but can beside the one it was unsure of,
    /// is sent.
    #[tokio::test]
    async fn the_window_check_counts_only_the_questions_asked_again() {
        let (first, second) = first_and_second(ProviderTier::Private, 512);
        let (first_calls, second_calls) = (first.calls.clone(), second.calls.clone());
        let (client, session) = client_with(vec![first, second], false).await;
        // tev1 reads each question's own `[p=…]`; clef reads the chunk's.
        let long_question = format!("Is this note long? [p=0.97] {}", "word ".repeat(400));
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"long": {"instructions": long_question},
                                 "short": {"instructions": "Is it short? [p=0.6]"}},
                   "chunks": [{"id": "a", "text": "a note [esc-p=0.9]"}],
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1, "{body}");
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.as_ref().unwrap();
        assert!(escalation.too_large_ids.is_empty(), "{body}");
        assert_eq!((escalation.answers, escalation.decided), (1, 1));
        let answer = &escalation.items[0].answers[0];
        assert_eq!(
            (answer.question.as_str(), answer.by.as_str()),
            ("short", "clef")
        );
        assert!((answer.p - 0.9).abs() < 1e-9, "{answer:?}");
    }

    /// A big batch has no line per chunk: the escalated lines say which
    /// chunks were too large, and the ones past the listed lines are named
    /// apart from the answered ones.
    #[tokio::test]
    async fn a_big_batch_names_its_chunks_too_large_for_the_second_model() {
        let (first, second) = first_and_second(ProviderTier::Private, 512);
        let second_calls = second.calls.clone();
        let (client, session) = client_with(vec![first, second], false).await;
        let chunks: Vec<serde_json::Value> = (0..30)
            .map(|i| {
                let text = if i % 10 == 9 {
                    format!("[state-p=0.6] {}", "word ".repeat(700))
                } else {
                    "note [state-p=0.6] [esc-p=0.95]".to_string()
                };
                json!({"id": format!("n{i:02}"), "text": text})
            })
            .collect();
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}}, "chunks": chunks,
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(second_calls.load(Ordering::SeqCst), 27, "{body}");
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!(escalation.too_large_ids, ["n09", "n19", "n29"]);
        assert_eq!(summary.low_confidence_ids, ["n09", "n19", "n29"]);
        assert!(
            body.contains("- n09: q yes P 0.60, too large for clef")
                && body.contains("- n00: q yes P 0.95, was yes P 0.60")
                && body.contains(" Too large for clef, so tev1's answer stands: n29.)"),
            "{body}"
        );
    }

    /// A second model that fails leaves the first answer standing, says so,
    /// and keeps the chunk on the low-confidence list.
    #[tokio::test]
    async fn a_failed_escalation_keeps_the_first_answer() {
        let (first, mut second) = first_and_second(ProviderTier::Private, 4_096);
        second.fail_all = true;
        let (client, session) = client_with(vec![first, second], false).await;
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"q": {"instructions": "yes?"}},
                   "chunks": marked_chunks(&[("a", 0.6, 0.9), ("b", 0.95, 0.9)]),
                   "model": "tev1", "escalate": {"model": "clef"}}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let escalation = summary.escalation.as_ref().unwrap();
        assert_eq!(
            (escalation.escalated, escalation.decided, escalation.errors),
            (1, 0, 1)
        );
        let answer = &escalation.items[0].answers[0];
        assert_eq!((answer.label.as_str(), answer.by.as_str()), ("yes", "tev1"));
        assert!(answer.error.is_some());
        assert_eq!(escalation.error_groups[0].chunk_ids, ["a"]);
        assert_eq!(summary.low_confidence_ids, ["a"]);
        assert!(
            body.contains("1 escalated answer failed, so tev1's answer stands for it.")
                && body.contains("Escalation failed (1 chunk): a: the decision server failed")
                && body.contains(
                    "- a: q P(yes) 0.60 review (escalation failed; tev1's answer stands)"
                ),
            "{body}"
        );
    }

    #[tokio::test]
    async fn escalate_is_refused_with_a_vote_its_own_first_model_or_a_bad_bound() {
        let (first, second) = first_and_second(ProviderTier::Private, 4_096);
        let first_calls = first.calls.clone();
        let (client, session) = client_with(vec![first, second], false).await;
        let ask = |extra: serde_json::Value| {
            let mut args = json!({"questions": {"q": {"instructions": "yes?"}},
                                  "chunks": [{"id": "a", "text": "a note"}]});
            for (key, value) in extra.as_object().unwrap() {
                args[key] = value.clone();
            }
            args
        };
        for (extra, expected) in [
            (
                json!({"models": ["tev1", "clef"], "escalate": {"model": "clef"}}),
                "does not go with a vote",
            ),
            (
                json!({"model": "tev1", "escalate": {"model": "tev1"}}),
                "the model that decides first",
            ),
            (
                json!({"escalate": {"model": "tev1"}}),
                "the model that decides first",
            ),
            (
                json!({"escalate": {"model": "clef", "below": 0}}),
                "above 0 and at most 1",
            ),
            (
                json!({"escalate": {"model": "clef", "below": 1.5}}),
                "above 0 and at most 1",
            ),
            (json!({"escalate": {"model": " "}}), "needs `model`"),
            (
                json!({"escalate": {"model": "gpt-giant"}}),
                "Nothing was decided. A decision model named gpt-giant is not set up.",
            ),
        ] {
            let result = call(&client, &session, "decide", ask(extra), enforced()).await;
            assert_eq!(result.is_error, Some(true));
            assert!(
                text(&result).contains(expected),
                "{expected}: {}",
                text(&result)
            );
        }
        assert_eq!(first_calls.load(Ordering::SeqCst), 0, "nothing was sent");
    }

    /// Without `escalate`, the result lists every chunk with a top answer
    /// under 0.8, uncapped, and `only_ids` decides exactly those again.
    #[tokio::test]
    async fn the_low_confidence_ids_are_all_listed_and_only_ids_decides_exactly_those() {
        let (first, second) = first_and_second(ProviderTier::Private, 4_096);
        let second_calls = second.calls.clone();
        let (client, session) = client_with(vec![first, second], false).await;
        // 60 chunks (past the per-chunk lines and the 40-id caps), every
        // third one unsure.
        let marks: Vec<(String, f64, f64)> = (0..60)
            .map(|i| {
                (
                    format!("n{i:02}"),
                    if i % 3 == 0 { 0.55 } else { 0.97 },
                    0.9,
                )
            })
            .collect();
        let marks: Vec<(&str, f64, f64)> = marks
            .iter()
            .map(|(id, a, b)| (id.as_str(), *a, *b))
            .collect();
        let chunks = marked_chunks(&marks);
        let question = json!({"q": {"instructions": "yes?", "threshold": 0.5}});
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": question, "chunks": chunks, "model": "tev1"}),
            enforced(),
        )
        .await;
        let body = text(&result);
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let expected: Vec<String> = (0..60).step_by(3).map(|i| format!("n{i:02}")).collect();
        assert_eq!(summary.low_confidence_ids, expected);
        assert_eq!(summary.low_confidence_below, LOW_CONFIDENCE);
        assert!(summary.escalation.is_none());
        assert!(
            body.contains(&format!(
                "Low confidence (a top answer under 0.80), 20 of 60 chunks: {}. To ask another \
                 model about exactly these, repeat this call with `only_ids`",
                expected.join(", ")
            )),
            "{body}"
        );

        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": question, "chunks": chunks, "model": "clef",
                   "only_ids": expected}),
            enforced(),
        )
        .await;
        let body = text(&result);
        assert_eq!(result.is_error, Some(false), "{body}");
        assert_eq!(second_calls.load(Ordering::SeqCst), 20, "exactly those");
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(summary.chunks, 20);
        assert!(summary.low_confidence_ids.is_empty(), "{body}");

        for (only, expected) in [
            (
                json!(["n00", "nope"]),
                "`nope`, which is not the id of a chunk",
            ),
            (json!(["n00", "n00"]), "names `n00` twice"),
        ] {
            let result = call(
                &client,
                &session,
                "decide",
                json!({"questions": question, "chunks": chunks, "only_ids": only}),
                enforced(),
            )
            .await;
            assert_eq!(result.is_error, Some(true));
            assert!(text(&result).contains(expected), "{}", text(&result));
        }
    }

    /// A batch too big for a line per chunk still gives each chunk's label of
    /// a choice: its ids under each answer, in option order, capped.
    #[tokio::test]
    async fn a_big_choice_lists_its_chunks_under_each_answer() {
        let local = model(
            entry(
                "tev1",
                ProviderTier::Private,
                true,
                DecisionAdapter::SystemOne,
            ),
            0.5,
        );
        let (client, session) = client_with(vec![local], false).await;
        let chunks: Vec<serde_json::Value> = (0..30)
            .map(|i| {
                // The fake puts P on the first option; under 1/3 the second
                // option and the third share the rest, and the second wins
                // the tie.
                let p = if i < 10 { 0.9 } else { 0.1 };
                json!({"id": format!("n{i:02}"), "text": format!("note [state-p={p}]")})
            })
            .collect();
        let result = call(
            &client,
            &session,
            "decide",
            json!({"questions": {"course": {"instructions": "Which?", "type": "choice",
                   "options": {"rrms": "Relapsing", "spms": "Secondary", "none": "Not stated"},
                   "threshold": 0.5, "pass_option": "rrms"}},
                   "chunks": chunks}),
            enforced(),
        )
        .await;
        let body = text(&result);
        let summary: DecideSummary =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        let ids = summary.questions[0].answer_ids.as_ref().unwrap();
        assert_eq!(ids.keys().collect::<Vec<_>>(), ["rrms", "spms", "none"]);
        assert_eq!(ids["rrms"].len(), 10);
        assert_eq!(ids["spms"].len(), 20);
        assert!(ids["none"].is_empty());
        assert!(body.contains("\n  rrms: n00, n01"), "{body}");
        assert!(body.contains("\n  spms: n10, n11"), "{body}");
        assert!(
            !body.contains("\n  none:"),
            "an empty answer is left out: {body}"
        );
    }

    /// Models reach for `threshold`, the word the questions use; for
    /// `escalate` it means `below`.
    #[test]
    fn escalate_reads_threshold_as_below_and_trims_its_model() {
        let params: DecideParams = serde_json::from_value(json!({
            "questions": {"q": {"instructions": "yes?"}},
            "chunks": [{"id": "a", "text": "x"}],
            "escalate": {"model": " clef ", "threshold": 0.6},
        }))
        .unwrap();
        assert_eq!(
            escalation_ask(&params).unwrap(),
            Some(EscalationAsk {
                model: "clef".into(),
                below: 0.6
            })
        );
    }

    /// The decide tool is offered in every chat, so what it costs the model to
    /// read is held: its description and its input schema together.
    #[test]
    fn the_decide_tool_stays_short() {
        let tools = DecisionsClient::get_tools();
        let decide = tools.iter().find(|t| t.name == "decide").unwrap();
        let description = decide.description.as_deref().unwrap_or_default();
        let schema = serde_json::to_string(&decide.input_schema).unwrap();
        // Measured 2026-11-12 with `escalate` and `only_ids`: 2,775 and 4,330.
        assert!(description.len() <= 2_800, "{}", description.len());
        assert!(schema.len() <= 4_400, "{}", schema.len());
    }

    #[test]
    fn only_ids_left_out_keeps_every_chunk() {
        let chunks = vec![
            DecisionChunk {
                id: "a".into(),
                context: DecisionContext::Text("x".into()),
            },
            DecisionChunk {
                id: "b".into(),
                context: DecisionContext::Text("y".into()),
            },
        ];
        let all = only_chunks(chunks.clone(), None).unwrap();
        assert_eq!(
            all.iter()
                .map(|(place, c)| (*place, c.id.as_str()))
                .collect::<Vec<_>>(),
            [(0, "a"), (1, "b")]
        );
        // A chunk keeps its place in the call, so the history numbers it as
        // the first pass did.
        let kept = only_chunks(chunks.clone(), Some(&["b".to_string()])).unwrap();
        assert_eq!(
            kept.iter()
                .map(|(place, c)| (*place, c.id.as_str()))
                .collect::<Vec<_>>(),
            [(1, "b")]
        );
        assert!(only_chunks(chunks, Some(&[]))
            .unwrap_err()
            .contains("names no chunk"));
    }
}
