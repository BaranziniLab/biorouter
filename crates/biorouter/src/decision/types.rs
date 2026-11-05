//! The types every decision passes through: what is asked ([`DecisionRequest`]),
//! what comes back ([`Decision`]), and the arithmetic that makes the two
//! comparable across adapters ([`confidence`], [`Threshold::passes`]).
//!
//! The question shapes and their validation descend from Albert Lee's prototype
//! in PR #469 (`agents/exit_criteria.rs`): typed kinds, `deny_unknown_fields` on
//! everything a caller writes, and the rule that a probability equal to the
//! threshold counts as at or above it. What changed is recorded in the design
//! (`decision-models` design §2): option descriptions are separate from option
//! names, a score may have 1 to 10 levels as the API allows, many named
//! questions share one state, and the whole distribution is kept rather than a
//! single value.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::privacy::ProviderTier;
use crate::providers::base::{ProviderUsage, Usage};

/// Questions per request. Ollama's System One limit; Baseten states none.
pub const MAX_QUESTIONS: usize = 64;
/// Options per choice question. Ollama's limit, and the number of letters the
/// logprob adapter can map options onto.
pub const MAX_CHOICE_OPTIONS: usize = 26;
/// A choice needs two options, or there is nothing to choose and confidence is
/// undefined.
pub const MIN_CHOICE_OPTIONS: usize = 2;
/// Score levels, as the System One API allows (lowest first, numbered from 0).
pub const MAX_SCORE_LEVELS: usize = 10;
pub const MAX_INSTRUCTIONS_CHARS: usize = 8_000;
pub const MAX_QUESTION_NAME_CHARS: usize = 64;
pub const MAX_OPTION_NAME_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 2_000;
pub const MAX_LEVEL_CHARS: usize = 400;
/// How far a reported distribution may stray from summing to 1 before it is
/// refused rather than renormalised.
const SUM_TOLERANCE: f64 = 0.02;
/// A probability this close below a threshold still counts as at it, so that
/// renormalisation's rounding cannot flip a gate.
const THRESHOLD_EPSILON: f64 = 1e-9;

/// What kind of answer a question takes.
///
/// Serialized externally tagged, `{"binary":{}}`, `{"choice":{"options":{...}}}`
/// and `{"score":{"levels":[...]}}`, which is the shape the design's monitor
/// criteria and the decide tool use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionKind {
    /// Yes or no. System One's `noul`.
    Binary {},
    /// One of named options. Each name maps to an optional description, which
    /// is what the model reads; a missing description sends the name itself.
    Choice {
        #[schema(value_type = HashMap<String, Option<String>>)]
        options: IndexMap<String, Option<String>>,
    },
    /// An ordered rubric of 1 to 10 levels, lowest first, numbered from 0.
    Score { levels: Vec<String> },
}

/// The kind of a question without its options, for capability lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKindTag {
    Binary,
    Choice,
    Score,
}

impl DecisionKind {
    pub fn tag(&self) -> DecisionKindTag {
        match self {
            Self::Binary {} => DecisionKindTag::Binary,
            Self::Choice { .. } => DecisionKindTag::Choice,
            Self::Score { .. } => DecisionKindTag::Score,
        }
    }

    /// The keys of [`Decision::probabilities`] for this kind, in order:
    /// `yes`/`no`, the option names, or the level indices `"0"`, `"1"`, ...
    pub fn outcome_keys(&self) -> Vec<String> {
        match self {
            Self::Binary {} => vec![YES.to_string(), NO.to_string()],
            Self::Choice { options } => options.keys().cloned().collect(),
            Self::Score { levels } => (0..levels.len()).map(|i| i.to_string()).collect(),
        }
    }

    /// The number of outcomes, k in the confidence formula.
    pub fn outcome_count(&self) -> usize {
        match self {
            Self::Binary {} => 2,
            Self::Choice { options } => options.len(),
            Self::Score { levels } => levels.len(),
        }
    }
}

pub const YES: &str = "yes";
pub const NO: &str = "no";

/// What "pass" means for one question. It acts on the probability of the
/// answer the caller wants, never on [`Decision::confidence`], which measures
/// how concentrated the distribution is and is not the probability of
/// anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Threshold {
    /// Binary: P(yes) at or above this.
    Yes(f64),
    /// Binary: P(no) at or above this.
    No(f64),
    /// Choice: P(option) at or above `p`.
    Option { name: String, p: f64 },
    /// Score: P(level >= `level`), the sum of the level probabilities from
    /// `level` up, at or above `p`. The fractional expected score is not used,
    /// because it hides a bimodal distribution.
    AtLeastLevel { level: usize, p: f64 },
}

impl Threshold {
    fn probability(&self) -> f64 {
        match self {
            Self::Yes(p) | Self::No(p) => *p,
            Self::Option { p, .. } | Self::AtLeastLevel { p, .. } => *p,
        }
    }

    /// The probability this threshold is compared against, read from a
    /// distribution keyed as [`DecisionKind::outcome_keys`].
    pub fn pass_probability(&self, probabilities: &IndexMap<String, f64>) -> f64 {
        match self {
            Self::Yes(_) => probabilities.get(YES).copied().unwrap_or(0.0),
            Self::No(_) => probabilities.get(NO).copied().unwrap_or(0.0),
            Self::Option { name, .. } => probabilities.get(name).copied().unwrap_or(0.0),
            Self::AtLeastLevel { level, .. } => probabilities
                .iter()
                .filter_map(|(key, p)| key.parse::<usize>().ok().map(|i| (i, *p)))
                .filter(|(i, _)| i >= level)
                .map(|(_, p)| p)
                .sum(),
        }
    }

    /// Whether the distribution passes. A probability equal to the threshold
    /// counts as at or above it (PR #469's noul rule).
    pub fn passes(&self, probabilities: &IndexMap<String, f64>) -> bool {
        self.pass_probability(probabilities) + THRESHOLD_EPSILON >= self.probability()
    }

    fn validate(&self, question: &str, kind: &DecisionKind) -> Result<(), String> {
        let p = self.probability();
        if !p.is_finite() || !(0.0..=1.0).contains(&p) {
            return Err(format!(
                "Question `{question}`: a threshold must be a probability between 0 and 1"
            ));
        }
        match (self, kind) {
            (Self::Yes(_) | Self::No(_), DecisionKind::Binary {}) => Ok(()),
            (Self::Option { name, .. }, DecisionKind::Choice { options }) => {
                if options.contains_key(name) {
                    Ok(())
                } else {
                    Err(format!(
                        "Question `{question}`: the threshold names `{name}`, which is not one of its options"
                    ))
                }
            }
            (Self::AtLeastLevel { level, .. }, DecisionKind::Score { levels }) => {
                if *level < levels.len() {
                    Ok(())
                } else {
                    Err(format!(
                        "Question `{question}`: the threshold names level {level}, but the levels are numbered 0 to {}",
                        levels.len() - 1
                    ))
                }
            }
            _ => Err(format!(
                "Question `{question}`: a `{}` threshold does not fit a {:?} question",
                match self {
                    Self::Yes(_) => "yes",
                    Self::No(_) => "no",
                    Self::Option { .. } => "option",
                    Self::AtLeastLevel { .. } => "at_least_level",
                },
                kind.tag()
            )),
        }
    }
}

/// One named question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Question {
    pub instructions: String,
    pub kind: DecisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Threshold>,
}

/// One message of a [`DecisionContext::Messages`] state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextMessage {
    /// `user`, `assistant` or `system`.
    pub role: String,
    pub content: String,
}

/// The state the questions are asked about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionContext {
    /// Plain text: a note, a file, a tool result.
    Text(String),
    /// Structured data. A top-level number, boolean or null is wrapped as
    /// `{"text": ...}` on the wire, which System One requires.
    Json(#[schema(value_type = Object)] Value),
    /// A conversation, sent as System One's `{"messages":[...]}` state.
    Messages(Vec<ContextMessage>),
}

impl DecisionContext {
    /// The state as plain text, for adapters that put it in a prompt.
    pub fn render_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Json(value) => match value {
                Value::String(text) => text.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            },
            Self::Messages(messages) => messages
                .iter()
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    }

    /// Bytes of the state as it would be rendered, for size checks and logs.
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Json(value) => value.to_string().len(),
            Self::Messages(messages) => messages
                .iter()
                .map(|m| m.role.len() + m.content.len())
                .sum(),
        }
    }
}

/// One decision request: up to [`MAX_QUESTIONS`] named questions over one
/// shared state. Every question is answered from the same state in one model
/// request where the adapter supports it (System One), so Baseten bills the
/// state once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionRequest {
    #[schema(value_type = HashMap<String, Question>)]
    pub questions: IndexMap<String, Question>,
    pub context: DecisionContext,
}

impl DecisionRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_questions(&self.questions)?;
        if let DecisionContext::Messages(messages) = &self.context {
            if messages.is_empty() {
                return Err("A messages state needs at least one message".to_string());
            }
            for message in messages {
                if !matches!(message.role.as_str(), "user" | "assistant" | "system") {
                    return Err(format!(
                        "A state message's role must be user, assistant or system, not `{}`",
                        message.role
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Validation shared by [`DecisionRequest::validate`] and batch calls, which
/// carry one question set over many states.
pub fn validate_questions(questions: &IndexMap<String, Question>) -> Result<(), String> {
    if questions.is_empty() || questions.len() > MAX_QUESTIONS {
        return Err(format!(
            "Ask between 1 and {MAX_QUESTIONS} questions per request, not {}",
            questions.len()
        ));
    }
    for (name, question) in questions {
        validate_question(name, question)?;
    }
    Ok(())
}

fn valid_question_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_QUESTION_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn validate_question(name: &str, question: &Question) -> Result<(), String> {
    if !valid_question_name(name) {
        return Err(format!(
            "Question name `{name}` must be 1 to {MAX_QUESTION_NAME_CHARS} letters, digits, `_` or `-`"
        ));
    }
    let instructions = question.instructions.trim();
    if instructions.is_empty() || question.instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err(format!(
            "Question `{name}`: write instructions of 1 to {MAX_INSTRUCTIONS_CHARS} characters"
        ));
    }
    match &question.kind {
        DecisionKind::Binary {} => {}
        DecisionKind::Choice { options } => {
            if !(MIN_CHOICE_OPTIONS..=MAX_CHOICE_OPTIONS).contains(&options.len()) {
                return Err(format!(
                    "Question `{name}`: a choice needs {MIN_CHOICE_OPTIONS} to {MAX_CHOICE_OPTIONS} options, not {}",
                    options.len()
                ));
            }
            for (option, description) in options {
                if option.trim().is_empty() || option.chars().count() > MAX_OPTION_NAME_CHARS {
                    return Err(format!(
                        "Question `{name}`: option names must be 1 to {MAX_OPTION_NAME_CHARS} characters"
                    ));
                }
                if description
                    .as_ref()
                    .is_some_and(|d| d.chars().count() > MAX_DESCRIPTION_CHARS)
                {
                    return Err(format!(
                        "Question `{name}`: option `{option}`'s description is over {MAX_DESCRIPTION_CHARS} characters"
                    ));
                }
            }
            // IndexMap keys are unique already; a case-blind duplicate would map
            // to one logprob letter's meaning twice and read as two options.
            let mut seen = std::collections::HashSet::new();
            for option in options.keys() {
                if !seen.insert(option.trim().to_lowercase()) {
                    return Err(format!(
                        "Question `{name}`: options `{option}` and another differ only in case or spacing"
                    ));
                }
            }
        }
        DecisionKind::Score { levels } => {
            if levels.is_empty() || levels.len() > MAX_SCORE_LEVELS {
                return Err(format!(
                    "Question `{name}`: a score needs 1 to {MAX_SCORE_LEVELS} levels, lowest first, not {}",
                    levels.len()
                ));
            }
            let mut seen = std::collections::HashSet::new();
            for level in levels {
                if level.trim().is_empty() || level.chars().count() > MAX_LEVEL_CHARS {
                    return Err(format!(
                        "Question `{name}`: score levels must be 1 to {MAX_LEVEL_CHARS} characters"
                    ));
                }
                if !seen.insert(level.trim()) {
                    return Err(format!(
                        "Question `{name}`: score level `{level}` appears twice"
                    ));
                }
            }
        }
    }
    if let Some(threshold) = &question.threshold {
        threshold.validate(name, &question.kind)?;
    }
    Ok(())
}

/// The answer a distribution points to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// `true` when P(yes) is at least one half.
    Binary {
        value: bool,
    },
    Choice {
        option: String,
    },
    /// The most probable level, plus the probability-weighted expected level
    /// (System One's fractional `score`). Gate on [`Threshold::AtLeastLevel`],
    /// never on `expected`.
    Score {
        level: usize,
        label: String,
        expected: f64,
    },
}

/// Where a decision's probabilities came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProbabilitySource {
    /// A decision model's own distribution (System One).
    Native,
    /// Token log-probabilities of a chat model, summed over each option's token
    /// variants and renormalised. `mass` is the share of probability that fell
    /// on option tokens before renormalising.
    Logprob { mass: f64 },
    /// A number the model wrote. Never calibrated. `reason` says why a logprob
    /// adapter fell back to this, when it did.
    Verbalized {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// Which adapter produced a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAdapter {
    SystemOne,
    Logprob,
    Verbalized,
}

/// Names one decision model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionModelRef {
    /// `baseten_decision` or `ollama_decision`.
    pub provider: String,
    pub model: String,
    /// The adapter to use, when the caller wants one other than the model's
    /// default (for example `logprob` for an Ollama yes/no checker served on
    /// `/api/chat`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<DecisionAdapter>,
}

impl DecisionModelRef {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            adapter: None,
        }
    }

    pub fn with_adapter(mut self, adapter: DecisionAdapter) -> Self {
        self.adapter = Some(adapter);
        self
    }

    /// `provider/model`, the spelling config and the CLI use.
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    /// Parse `provider/model`. The model may itself contain `/`
    /// (`baseten_decision/decision-models/...`).
    pub fn parse(text: &str) -> Option<Self> {
        let (provider, model) = text.trim().split_once('/')?;
        (!provider.is_empty() && !model.is_empty()).then(|| Self::new(provider, model))
    }
}

/// Token usage of the model request a decision came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

/// Usage of two requests together. A count either side reported is kept; one
/// neither reported stays unknown.
impl std::ops::Add for DecisionUsage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let sum = |a: Option<i64>, b: Option<i64>| match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        Self {
            input_tokens: sum(self.input_tokens, other.input_tokens),
            output_tokens: sum(self.output_tokens, other.output_tokens),
        }
    }
}

impl DecisionUsage {
    /// The figure the usage ledger and reply budget take. The context gauge
    /// must not: a decision is not part of the chat's window.
    pub fn to_provider_usage(self, provider: &str, model: &str) -> ProviderUsage {
        let clamp = |n: Option<i64>| n.and_then(|n| i32::try_from(n).ok());
        let input = clamp(self.input_tokens);
        let output = clamp(self.output_tokens);
        let mut usage = ProviderUsage::new(
            model.to_string(),
            Usage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input.zip(output).and_then(|(a, b)| a.checked_add(b)),
                ..Usage::default()
            },
        );
        usage.provider = Some(provider.to_string());
        usage
    }
}

/// One answered question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub question: String,
    pub answer: Answer,
    /// Sums to 1. Keyed `yes`/`no`, by option name, or by level index.
    #[schema(value_type = HashMap<String, f64>)]
    pub probabilities: IndexMap<String, f64>,
    /// How concentrated the distribution is, computed by BioRouter the same way
    /// for every adapter: (max p - 1/k) / (1 - 1/k) for k outcomes, which is
    /// |2p - 1| for a binary question. 0 is uniform, 1 is certain. Not the
    /// probability of the answer. A native value, when the model sent one, is
    /// in `raw`.
    pub confidence: f64,
    /// Set when the question had a threshold.
    pub passed: Option<bool>,
    /// The probability the threshold was compared against, when there was one.
    pub pass_probability: Option<f64>,
    pub source: ProbabilitySource,
    /// True only for a native distribution, or for logprobs whose option tokens
    /// held at least [`CALIBRATED_LOGPROB_MASS`] of the probability. Even then
    /// these are model probabilities that have not been validated on the
    /// caller's data.
    pub calibrated: bool,
    pub model: DecisionModelRef,
    pub tier: ProviderTier,
    /// The model request this decision came from. Decisions that share one
    /// share its `usage` and `latencyMs`, so sum usage per request id.
    pub request_id: String,
    pub latency_ms: u64,
    pub usage: DecisionUsage,
    /// What the backend returned for this question, unmodified.
    #[schema(value_type = Object)]
    pub raw: Value,
}

/// Logprob mass at or above which a logprob decision counts as calibrated.
pub const CALIBRATED_LOGPROB_MASS: f64 = 0.9;

/// One question's distribution as an adapter produced it, before BioRouter
/// derives the answer, confidence and pass.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionVerdict {
    pub question: String,
    /// Keyed as [`DecisionKind::outcome_keys`]. Missing keys count as 0; the
    /// service renormalises.
    pub probabilities: IndexMap<String, f64>,
    pub source: ProbabilitySource,
    pub calibrated: bool,
    /// The backend's answer (an option name), used only to break a tie.
    pub reported_choice: Option<String>,
    pub raw: Value,
}

/// What one adapter call returns.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelVerdicts {
    pub verdicts: Vec<QuestionVerdict>,
    pub usage: DecisionUsage,
    /// The model id the backend says served the request (Baseten may answer
    /// with a different id from the slug sent).
    pub served_model: Option<String>,
}

/// The concentration measure, the same for every adapter.
pub fn confidence(probabilities: &IndexMap<String, f64>) -> f64 {
    let k = probabilities.len();
    if k <= 1 {
        // One outcome is a forced answer: the distribution is certain and
        // uniform at once. Reported as certain, because p(max) is 1.
        return 1.0;
    }
    let max = probabilities.values().copied().fold(0.0_f64, f64::max);
    let uniform = 1.0 / k as f64;
    ((max - uniform) / (1.0 - uniform)).clamp(0.0, 1.0)
}

/// Check a reported distribution against the question and renormalise it to
/// sum to exactly 1, in outcome order. Unknown keys, values outside [0, 1], a
/// sum far from 1 (for a native distribution) or a zero sum are refused.
pub fn normalize_distribution(
    question: &str,
    kind: &DecisionKind,
    reported: &IndexMap<String, f64>,
    require_unit_sum: bool,
) -> Result<IndexMap<String, f64>, String> {
    let keys = kind.outcome_keys();
    for (key, p) in reported {
        if !keys.iter().any(|k| k == key) {
            return Err(format!(
                "Question `{question}`: the model returned an unknown outcome `{key}`"
            ));
        }
        if !p.is_finite() || *p < 0.0 || *p > 1.0 + 1e-9 {
            return Err(format!(
                "Question `{question}`: the model returned a probability outside 0 to 1"
            ));
        }
    }
    let sum: f64 = keys
        .iter()
        .map(|k| reported.get(k).copied().unwrap_or(0.0))
        .sum();
    if sum <= 0.0 {
        return Err(format!(
            "Question `{question}`: the model's probabilities sum to zero"
        ));
    }
    if require_unit_sum && (sum - 1.0).abs() > SUM_TOLERANCE {
        return Err(format!(
            "Question `{question}`: the model's probabilities sum to {sum:.3}, not 1"
        ));
    }
    Ok(keys
        .into_iter()
        .map(|k| {
            let p = reported.get(&k).copied().unwrap_or(0.0) / sum;
            (k, p)
        })
        .collect())
}

/// The answer a normalised distribution points to. Ties go to the backend's
/// own answer when it is among the tied outcomes, else to the first in order.
pub fn answer_for(
    kind: &DecisionKind,
    probabilities: &IndexMap<String, f64>,
    reported_choice: Option<&str>,
) -> Answer {
    let max = probabilities.values().copied().fold(f64::MIN, f64::max);
    let tied = |key: &str| {
        probabilities
            .get(key)
            .is_some_and(|p| (max - p).abs() <= THRESHOLD_EPSILON)
    };
    let top = reported_choice
        .filter(|choice| tied(choice))
        .map(str::to_string)
        .or_else(|| {
            probabilities
                .iter()
                .find(|(key, _)| tied(key))
                .map(|(key, _)| key.clone())
        })
        .unwrap_or_default();
    match kind {
        DecisionKind::Binary {} => Answer::Binary {
            value: probabilities.get(YES).copied().unwrap_or(0.0) >= 0.5,
        },
        DecisionKind::Choice { .. } => Answer::Choice { option: top },
        DecisionKind::Score { levels } => {
            let level = top.parse::<usize>().unwrap_or(0);
            let expected = probabilities
                .iter()
                .filter_map(|(key, p)| key.parse::<usize>().ok().map(|i| i as f64 * p))
                .sum();
            Answer::Score {
                level,
                label: levels.get(level).cloned().unwrap_or_default(),
                expected,
            }
        }
    }
}

/// Everything the service needs to turn one verdict into a [`Decision`].
pub struct DecisionMeta<'a> {
    pub model: &'a DecisionModelRef,
    pub tier: ProviderTier,
    pub request_id: &'a str,
    pub latency_ms: u64,
    pub usage: DecisionUsage,
}

/// Build the decision for `question` from an adapter's verdict: validate and
/// renormalise the distribution, then derive the answer, confidence and pass
/// the same way for every adapter.
pub fn decision_from_verdict(
    name: &str,
    question: &Question,
    verdict: QuestionVerdict,
    meta: &DecisionMeta<'_>,
) -> Result<Decision, String> {
    let require_unit_sum = matches!(verdict.source, ProbabilitySource::Native);
    let probabilities = normalize_distribution(
        name,
        &question.kind,
        &verdict.probabilities,
        require_unit_sum,
    )?;
    let answer = answer_for(
        &question.kind,
        &probabilities,
        verdict.reported_choice.as_deref(),
    );
    let pass_probability = question
        .threshold
        .as_ref()
        .map(|t| t.pass_probability(&probabilities));
    let passed = question
        .threshold
        .as_ref()
        .map(|t| t.passes(&probabilities));
    Ok(Decision {
        question: name.to_string(),
        answer,
        confidence: confidence(&probabilities),
        probabilities,
        passed,
        pass_probability,
        source: verdict.source,
        calibrated: verdict.calibrated,
        model: meta.model.clone(),
        tier: meta.tier,
        request_id: meta.request_id.to_string(),
        latency_ms: meta.latency_ms,
        usage: meta.usage,
        raw: verdict.raw,
    })
}

/// Why a decision call happened, for the decision log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOrigin {
    /// A decision model bound as a chat's main model.
    Chat,
    Monitor,
    Tool,
    Gate,
    #[default]
    Adhoc,
}

/// One chunk of a batch: an id the caller can find it by, and its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionChunk {
    pub id: String,
    pub context: DecisionContext,
}

/// One chunk's result in a batch. Exactly one of `decisions` and `error` is
/// set; a failed chunk never fails the batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChunkOutcome {
    pub chunk_id: String,
    pub decisions: Option<Vec<Decision>>,
    pub error: Option<String>,
}

/// The ids of the chunks that passed each question in a batch, for the
/// decide tool's summary. A question no chunk passed maps to an empty list.
pub fn passing_ids(outcomes: &[ChunkOutcome]) -> BTreeMap<String, Vec<String>> {
    let mut passing: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for outcome in outcomes {
        for decision in outcome.decisions.iter().flatten() {
            let entry = passing.entry(decision.question.clone()).or_default();
            if decision.passed == Some(true) {
                entry.push(outcome.chunk_id.clone());
            }
        }
    }
    passing
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binary(threshold: Option<Threshold>) -> Question {
        Question {
            instructions: "Is the form reached?".into(),
            kind: DecisionKind::Binary {},
            threshold,
        }
    }

    fn choice() -> DecisionKind {
        DecisionKind::Choice {
            options: IndexMap::from([
                ("rrms".to_string(), Some("Relapsing-remitting".to_string())),
                ("spms".to_string(), None),
                ("none".to_string(), Some("Not stated".to_string())),
            ]),
        }
    }

    fn dist(pairs: &[(&str, f64)]) -> IndexMap<String, f64> {
        pairs.iter().map(|(k, p)| (k.to_string(), *p)).collect()
    }

    fn request(questions: Vec<(&str, Question)>) -> DecisionRequest {
        DecisionRequest {
            questions: questions
                .into_iter()
                .map(|(n, q)| (n.to_string(), q))
                .collect(),
            context: DecisionContext::Text("state".into()),
        }
    }

    #[test]
    fn the_wire_shapes_match_the_design() {
        let question: Question = serde_json::from_value(json!({
            "instructions": "Has the agent reached the consent form page?",
            "kind": {"binary": {}},
            "threshold": {"yes": 0.85}
        }))
        .unwrap();
        assert_eq!(question.kind, DecisionKind::Binary {});
        assert_eq!(question.threshold, Some(Threshold::Yes(0.85)));

        let score: DecisionKind =
            serde_json::from_value(json!({"score": {"levels": ["low", "high"]}})).unwrap();
        assert_eq!(score.tag(), DecisionKindTag::Score);
        let at_least: Threshold =
            serde_json::from_value(json!({"at_least_level": {"level": 1, "p": 0.8}})).unwrap();
        assert_eq!(at_least, Threshold::AtLeastLevel { level: 1, p: 0.8 });

        let ctx: DecisionContext =
            serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]}))
                .unwrap();
        assert!(matches!(ctx, DecisionContext::Messages(m) if m.len() == 1));

        // Unknown fields are refused everywhere a caller writes.
        assert!(serde_json::from_value::<Question>(json!({
            "instructions": "x", "kind": {"binary": {}}, "extra": 1
        }))
        .is_err());
        assert!(serde_json::from_value::<DecisionKind>(json!({"binary": {"x": 1}})).is_err());
    }

    #[test]
    fn decisions_serialize_camel_case_with_snake_case_enums() {
        let model = DecisionModelRef::new("baseten_decision", "joey-1");
        let meta = DecisionMeta {
            model: &model,
            tier: ProviderTier::Public,
            request_id: "r1",
            latency_ms: 12,
            usage: DecisionUsage::default(),
        };
        let verdict = QuestionVerdict {
            question: "q".into(),
            probabilities: dist(&[("yes", 0.9), ("no", 0.1)]),
            source: ProbabilitySource::Logprob { mass: 0.95 },
            calibrated: true,
            reported_choice: None,
            raw: json!({}),
        };
        let decision =
            decision_from_verdict("q", &binary(Some(Threshold::Yes(0.8))), verdict, &meta).unwrap();
        let value = serde_json::to_value(&decision).unwrap();
        assert_eq!(value["latencyMs"], 12);
        assert_eq!(value["requestId"], "r1");
        assert_eq!(value["passProbability"], 0.9);
        assert_eq!(value["source"], json!({"kind": "logprob", "mass": 0.95}));
        assert_eq!(value["answer"], json!({"type": "binary", "value": true}));
        assert_eq!(value["tier"], "public");
    }

    #[test]
    fn validation_accepts_the_api_ranges_and_refuses_the_rest() {
        assert!(request(vec![("q", binary(None))]).validate().is_ok());
        assert!(request(vec![]).validate().is_err());
        let too_many: Vec<_> = (0..=MAX_QUESTIONS).map(|_| ("q", binary(None))).collect();
        // Same key collapses in an IndexMap, so build distinct names.
        let mut questions = IndexMap::new();
        for (i, (_, q)) in too_many.into_iter().enumerate() {
            questions.insert(format!("q{i}"), q);
        }
        assert!(DecisionRequest {
            questions,
            context: DecisionContext::Text("s".into())
        }
        .validate()
        .is_err());

        // Names are map keys on the wire: keep them plain.
        assert!(request(vec![("has ms", binary(None))]).validate().is_err());
        // Instructions are required.
        let mut empty = binary(None);
        empty.instructions = "  ".into();
        assert!(request(vec![("q", empty)]).validate().is_err());

        // A one-level score is allowed, as the API allows it; eleven are not.
        let score = |n: usize| Question {
            instructions: "Rate it".into(),
            kind: DecisionKind::Score {
                levels: (0..n).map(|i| format!("level {i}")).collect(),
            },
            threshold: None,
        };
        assert!(request(vec![("s", score(1))]).validate().is_ok());
        assert!(request(vec![("s", score(10))]).validate().is_ok());
        assert!(request(vec![("s", score(0))]).validate().is_err());
        assert!(request(vec![("s", score(11))]).validate().is_err());

        // A choice needs two options; case-blind duplicates are refused.
        let one = Question {
            instructions: "Pick".into(),
            kind: DecisionKind::Choice {
                options: IndexMap::from([("a".to_string(), None)]),
            },
            threshold: None,
        };
        assert!(request(vec![("c", one)]).validate().is_err());
        let dup = Question {
            instructions: "Pick".into(),
            kind: DecisionKind::Choice {
                options: IndexMap::from([("Yes".to_string(), None), ("yes".to_string(), None)]),
            },
            threshold: None,
        };
        assert!(request(vec![("c", dup)]).validate().is_err());
    }

    #[test]
    fn a_threshold_must_fit_its_question() {
        let ok = Question {
            instructions: "Course?".into(),
            kind: choice(),
            threshold: Some(Threshold::Option {
                name: "rrms".into(),
                p: 0.9,
            }),
        };
        assert!(request(vec![("c", ok.clone())]).validate().is_ok());
        let mut unknown = ok.clone();
        unknown.threshold = Some(Threshold::Option {
            name: "ppms".into(),
            p: 0.9,
        });
        assert!(request(vec![("c", unknown)]).validate().is_err());
        let mut wrong_kind = ok.clone();
        wrong_kind.threshold = Some(Threshold::Yes(0.5));
        assert!(request(vec![("c", wrong_kind)]).validate().is_err());
        assert!(request(vec![("b", binary(Some(Threshold::Yes(1.5))))])
            .validate()
            .is_err());
        assert!(request(vec![("b", binary(Some(Threshold::Yes(f64::NAN))))])
            .validate()
            .is_err());
        let level = Question {
            instructions: "Rate".into(),
            kind: DecisionKind::Score {
                levels: vec!["a".into(), "b".into()],
            },
            threshold: Some(Threshold::AtLeastLevel { level: 2, p: 0.5 }),
        };
        assert!(request(vec![("s", level)]).validate().is_err());
    }

    #[test]
    fn confidence_is_concentration_not_the_chosen_probability() {
        // Binary: |2p - 1|.
        for p in [0.0, 0.25, 0.5, 0.8, 1.0] {
            let c = confidence(&dist(&[("yes", p), ("no", 1.0 - p)]));
            assert!((c - (2.0 * p - 1.0).abs()).abs() < 1e-12, "p={p} c={c}");
        }
        // Uniform over k is 0, certain is 1.
        assert_eq!(
            confidence(&dist(&[("a", 0.25), ("b", 0.25), ("c", 0.25), ("d", 0.25)])),
            0.0
        );
        assert_eq!(
            confidence(&dist(&[("a", 1.0), ("b", 0.0), ("c", 0.0)])),
            1.0
        );
        // k = 4, max 0.6: (0.6 - 0.25) / 0.75.
        let c = confidence(&dist(&[("a", 0.6), ("b", 0.2), ("c", 0.1), ("d", 0.1)]));
        assert!((c - 0.35 / 0.75).abs() < 1e-12);
        // The System One `choice` example: P(chosen) 0.6 over two options is
        // confidence 0.2, not 0.6.
        let c = confidence(&dist(&[("review", 0.6), ("continue", 0.4)]));
        assert!((c - 0.2).abs() < 1e-12);
        // One forced outcome.
        assert_eq!(confidence(&dist(&[("0", 1.0)])), 1.0);
    }

    #[test]
    fn thresholds_act_on_the_desired_answer_and_include_equality() {
        let yes_no = dist(&[("yes", 0.8), ("no", 0.2)]);
        assert!(Threshold::Yes(0.8).passes(&yes_no));
        assert!(!Threshold::Yes(0.81).passes(&yes_no));
        assert!(Threshold::No(0.2).passes(&yes_no));
        assert!(!Threshold::No(0.5).passes(&yes_no));

        let course = dist(&[("rrms", 0.55), ("spms", 0.4), ("none", 0.05)]);
        let pick = Threshold::Option {
            name: "spms".into(),
            p: 0.4,
        };
        assert!(pick.passes(&course));
        assert_eq!(pick.pass_probability(&course), 0.4);

        // Score: P(level >= k), so a bimodal distribution whose expected level
        // is 1.0 still fails "at least level 2 with p 0.6".
        let bimodal = dist(&[("0", 0.5), ("1", 0.0), ("2", 0.5)]);
        let at_least_two = Threshold::AtLeastLevel { level: 2, p: 0.6 };
        assert!(!at_least_two.passes(&bimodal));
        assert!(Threshold::AtLeastLevel { level: 1, p: 0.5 }.passes(&bimodal));
        assert!(Threshold::AtLeastLevel { level: 0, p: 1.0 }.passes(&bimodal));
    }

    #[test]
    fn distributions_are_checked_then_renormalised() {
        let kind = choice();
        let ok = normalize_distribution(
            "c",
            &kind,
            &dist(&[("rrms", 0.6), ("spms", 0.2), ("none", 0.2)]),
            true,
        )
        .unwrap();
        assert_eq!(ok.keys().collect::<Vec<_>>(), ["rrms", "spms", "none"]);
        // Missing outcomes count as 0, and the order is the question's.
        let partial =
            normalize_distribution("c", &kind, &dist(&[("none", 0.5), ("rrms", 0.5)]), true)
                .unwrap();
        assert_eq!(partial["spms"], 0.0);
        assert_eq!(partial.keys().next().unwrap(), "rrms");
        // Unknown option, out of range, far from 1, or zero: refused.
        assert!(normalize_distribution("c", &kind, &dist(&[("ppms", 1.0)]), true).is_err());
        assert!(normalize_distribution("c", &kind, &dist(&[("rrms", 1.5)]), false).is_err());
        assert!(normalize_distribution("c", &kind, &dist(&[("rrms", 0.5)]), true).is_err());
        assert!(normalize_distribution("c", &kind, &dist(&[("rrms", 0.0)]), false).is_err());
        // A logprob mass below 1 is renormalised rather than refused.
        let lp = normalize_distribution("c", &kind, &dist(&[("rrms", 0.3), ("spms", 0.3)]), false)
            .unwrap();
        assert!((lp["rrms"] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn answers_come_from_the_distribution() {
        let score = DecisionKind::Score {
            levels: vec!["none".into(), "some".into(), "all".into()],
        };
        let p = dist(&[("0", 0.2), ("1", 0.3), ("2", 0.5)]);
        let Answer::Score {
            level,
            label,
            expected,
        } = answer_for(&score, &p, None)
        else {
            panic!("a score answer");
        };
        assert_eq!((level, label.as_str()), (2, "all"));
        assert!((expected - 1.3).abs() < 1e-12);

        // A tie goes to the backend's own answer when it is among the tied.
        let tie = dist(&[("rrms", 0.5), ("spms", 0.5), ("none", 0.0)]);
        assert_eq!(
            answer_for(&choice(), &tie, Some("spms")),
            Answer::Choice {
                option: "spms".into()
            }
        );
        assert_eq!(
            answer_for(&choice(), &tie, Some("none")),
            Answer::Choice {
                option: "rrms".into()
            }
        );
        assert_eq!(
            answer_for(
                &DecisionKind::Binary {},
                &dist(&[("yes", 0.5), ("no", 0.5)]),
                None
            ),
            Answer::Binary { value: true }
        );
    }

    #[test]
    fn model_refs_parse_with_slashes_in_the_model() {
        let r = DecisionModelRef::parse("baseten_decision/decision-models/baseten-joey-2b-latest")
            .unwrap();
        assert_eq!(r.provider, "baseten_decision");
        assert_eq!(r.model, "decision-models/baseten-joey-2b-latest");
        assert!(DecisionModelRef::parse("nothing").is_none());
        assert!(DecisionModelRef::parse("/x").is_none());
    }

    #[test]
    fn usage_maps_to_the_ledger_shape() {
        let usage = DecisionUsage {
            input_tokens: Some(42),
            output_tokens: Some(1),
        };
        let ledger = usage.to_provider_usage("baseten_decision", "joey-1");
        assert_eq!(ledger.usage.total_tokens, Some(43));
        assert_eq!(ledger.provider.as_deref(), Some("baseten_decision"));
        assert_eq!(
            usage
                + DecisionUsage {
                    input_tokens: None,
                    output_tokens: Some(2)
                },
            DecisionUsage {
                input_tokens: Some(42),
                output_tokens: Some(3)
            }
        );
    }
}
