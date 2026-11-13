//! G-1: `decide` with `models`, a vote of 2 to 5 decision models.
//!
//! Every chunk is asked of each model (each model's run is an ordinary
//! `decide_batch`, so its decisions are logged one row per model per chunk, all
//! under the call's one batch id). The result says, per question, each model's
//! answer counts, the majority answer's counts with ties counted as ties, how
//! often the models agreed, and which chunks they disagreed on with each
//! model's answer. Like a single-model result it stays an aggregate: a big
//! batch lists capped ids, and only a small one gets a line per chunk.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{
    answer_key, capped, error_groups, id_cap, kind_word, pass_on, threshold_p, tier_word,
    PER_CHUNK_CELLS, PER_CHUNK_LINES,
};
use crate::decision::{Answer, ChunkOutcome, Decision, DecisionModelRef, Question};
use crate::privacy::ProviderTier;
use crate::providers::decision::catalog::decision_model_display_label;

/// Models in one vote.
pub const MIN_VOTE_MODELS: usize = 2;
pub const MAX_VOTE_MODELS: usize = 5;

/// One model's run in a vote.
pub struct ModelRun {
    pub model: DecisionModelRef,
    pub tier: Option<ProviderTier>,
    /// The batch's outcomes, or why the model answered nothing at all.
    pub outcomes: Result<Vec<ChunkOutcome>, String>,
}

/// What a vote returns as structured content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteSummary {
    /// Always `decide_vote`.
    pub kind: String,
    pub batch_id: String,
    pub chunks: usize,
    pub elapsed_ms: u64,
    pub models: Vec<VoteModel>,
    pub questions: Vec<VoteQuestion>,
}

/// One model of a vote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteModel {
    pub model: DecisionModelRef,
    /// The name a person reads (an alias for a Baseten model).
    pub label: String,
    pub tier: Option<ProviderTier>,
    pub decided: usize,
    pub errors: usize,
    /// Why this model answered nothing, when it did not.
    pub error: Option<String>,
}

/// One question's vote over the batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteQuestion {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub threshold: Option<f64>,
    pub pass_on: Option<String>,
    /// Per model, in `models` order: how many chunks it gave each answer.
    pub per_model: Vec<IndexMap<String, usize>>,
    /// How many chunks have each answer as their majority.
    pub majority: IndexMap<String, usize>,
    /// Chunks whose answers tied.
    pub ties: usize,
    /// Chunks at least two models answered.
    pub compared: usize,
    /// Of those, chunks every answering model gave the same answer.
    pub agreed: usize,
    /// Chunks the models did not all agree on (capped).
    pub disagreements: Vec<VoteChunk>,
    pub more_disagreements: usize,
    /// Per answer, the chunks whose majority it is (capped).
    pub majority_ids: IndexMap<String, Vec<String>>,
}

/// One chunk's votes on one question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteChunk {
    pub chunk_id: String,
    /// Per model label, its answer; a model that did not answer is absent.
    pub votes: IndexMap<String, String>,
    /// The majority answer, or `tie (yes 2, no 2)`.
    pub majority: String,
}

/// The answer one decision casts: "yes" or "no" for a question with a pass (a
/// threshold, or a yes_no), else the answer itself.
fn vote_of(question: &Question, decision: &Decision) -> String {
    match (&question.threshold, &decision.answer) {
        (Some(_), _) => if decision.passed == Some(true) {
            "yes"
        } else {
            "no"
        }
        .to_string(),
        (None, Answer::Binary { value }) => if *value { "yes" } else { "no" }.to_string(),
        (None, _) => answer_key(decision),
    }
}

/// The majority of `votes`, or `None` with the tied counts.
fn majority_of(votes: &IndexMap<String, String>) -> Result<String, String> {
    let mut counts: IndexMap<&str, usize> = IndexMap::new();
    for vote in votes.values() {
        *counts.entry(vote.as_str()).or_default() += 1;
    }
    let top = counts.values().copied().max().unwrap_or(0);
    let leaders: Vec<&str> = counts
        .iter()
        .filter(|(_, n)| **n == top)
        .map(|(vote, _)| *vote)
        .collect();
    match leaders.as_slice() {
        [one] => Ok((*one).to_string()),
        _ => Err(format!(
            "tie ({})",
            counts
                .iter()
                .map(|(vote, n)| format!("{vote} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The vote's models as the result names them: each one's bare name (a
/// Baseten model's alias), which `models` also takes, unless two of them share
/// it, when both keep their provider/model spelling.
fn labels(runs: &[ModelRun]) -> Vec<String> {
    let full: Vec<String> = runs
        .iter()
        .map(|run| decision_model_display_label(&run.model))
        .collect();
    let bare: Vec<String> = full
        .iter()
        .map(|label| match label.split_once('/') {
            Some((_, name)) => name.to_string(),
            None => label.clone(),
        })
        .collect();
    full.into_iter()
        .zip(&bare)
        .map(|(full, short)| {
            if bare.iter().filter(|other| *other == short).count() == 1 {
                short.clone()
            } else {
                full
            }
        })
        .collect()
}

/// The vote's aggregate. Pure, so it can be tested without a model.
pub fn summarize_vote(
    questions: &IndexMap<String, Question>,
    chunk_ids: &[String],
    runs: &[ModelRun],
    batch_id: &str,
    elapsed_ms: u64,
) -> VoteSummary {
    let labels = labels(runs);
    let models = runs
        .iter()
        .zip(&labels)
        .map(|(run, label)| {
            let (decided, errors, error) = match &run.outcomes {
                Ok(outcomes) => {
                    let decided = outcomes.iter().filter(|o| o.decisions.is_some()).count();
                    (decided, outcomes.len() - decided, None)
                }
                Err(why) => (0, chunk_ids.len(), Some(why.clone())),
            };
            VoteModel {
                model: run.model.clone(),
                label: label.clone(),
                tier: run.tier,
                decided,
                errors,
                error,
            }
        })
        .collect();
    let cap = id_cap(questions.len());
    let questions = questions
        .iter()
        .map(|(name, question)| vote_question(name, question, chunk_ids, runs, &labels, cap))
        .collect();
    VoteSummary {
        kind: "decide_vote".to_string(),
        batch_id: batch_id.to_string(),
        chunks: chunk_ids.len(),
        elapsed_ms,
        models,
        questions,
    }
}

/// One chunk's answers, by model label.
type Votes = IndexMap<String, String>;
/// How many chunks gave each answer.
type Counts = IndexMap<String, usize>;

/// Per chunk (in `chunk_ids` order), each model's answer to `name`, by label;
/// and per model, how many chunks gave each answer.
fn cast_votes(
    name: &str,
    question: &Question,
    chunk_ids: &[String],
    runs: &[ModelRun],
    labels: &[String],
) -> (Vec<Votes>, Vec<Counts>) {
    let mut per_chunk: Vec<IndexMap<String, String>> = vec![IndexMap::new(); chunk_ids.len()];
    let mut per_model = Vec::with_capacity(runs.len());
    for (run, label) in runs.iter().zip(labels) {
        let mut counts: IndexMap<String, usize> = IndexMap::new();
        for outcome in run.outcomes.as_deref().unwrap_or_default() {
            let Some(index) = chunk_ids.iter().position(|id| *id == outcome.chunk_id) else {
                continue;
            };
            let Some(decision) = outcome
                .decisions
                .as_ref()
                .and_then(|d| d.iter().find(|d| d.question == name))
            else {
                continue;
            };
            let vote = vote_of(question, decision);
            *counts.entry(vote.clone()).or_default() += 1;
            per_chunk[index].insert(label.clone(), vote);
        }
        per_model.push(counts);
    }
    (per_chunk, per_model)
}

/// One question's vote over the batch.
fn vote_question(
    name: &str,
    question: &Question,
    chunk_ids: &[String],
    runs: &[ModelRun],
    labels: &[String],
    cap: usize,
) -> VoteQuestion {
    let (per_chunk, per_model) = cast_votes(name, question, chunk_ids, runs, labels);
    let mut majority: IndexMap<String, usize> = IndexMap::new();
    let mut majority_ids: IndexMap<String, Vec<String>> = IndexMap::new();
    let (mut ties, mut compared, mut agreed) = (0, 0, 0);
    let mut disagreements = Vec::new();
    for (chunk_id, votes) in chunk_ids.iter().zip(per_chunk) {
        if votes.is_empty() {
            continue;
        }
        let verdict = majority_of(&votes);
        match &verdict {
            Ok(answer) => {
                *majority.entry(answer.clone()).or_default() += 1;
                majority_ids
                    .entry(answer.clone())
                    .or_default()
                    .push(chunk_id.clone());
            }
            Err(_) => ties += 1,
        }
        if votes.len() < 2 {
            continue;
        }
        compared += 1;
        let first = votes.values().next();
        if votes.values().all(|vote| Some(vote) == first) {
            agreed += 1;
        } else {
            disagreements.push(VoteChunk {
                chunk_id: chunk_id.clone(),
                votes,
                majority: verdict.unwrap_or_else(|tie| tie),
            });
        }
    }
    let more_disagreements = disagreements.len().saturating_sub(cap);
    disagreements.truncate(cap);
    VoteQuestion {
        name: name.to_string(),
        kind: kind_word(question.kind.tag()).to_string(),
        threshold: threshold_p(question.threshold.as_ref()),
        pass_on: pass_on(question.threshold.as_ref()),
        per_model,
        majority,
        ties,
        compared,
        agreed,
        disagreements,
        more_disagreements,
        majority_ids: majority_ids
            .into_iter()
            .map(|(answer, ids)| (answer, capped(ids, cap).0))
            .collect(),
    }
}

fn counts_line(counts: &IndexMap<String, usize>) -> String {
    if counts.is_empty() {
        return "no answers".to_string();
    }
    counts
        .iter()
        .map(|(answer, n)| format!("{answer} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn votes_line(votes: &IndexMap<String, String>) -> String {
    votes
        .iter()
        .map(|(model, vote)| format!("{model} {vote}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The text the agent reads for a vote.
pub fn render_vote(
    summary: &VoteSummary,
    questions: &IndexMap<String, Question>,
    chunk_ids: &[String],
    runs: &[ModelRun],
) -> String {
    let models = summary
        .models
        .iter()
        .map(|m| match m.tier {
            Some(tier) => format!("{} ({})", m.label, tier_word(tier)),
            None => m.label.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        "decide: {} chunk{} x {} question{}, a vote of {} models ({models}) in {:.1} s.",
        summary.chunks,
        plural(summary.chunks),
        questions.len(),
        plural(questions.len()),
        summary.models.len(),
        summary.elapsed_ms as f64 / 1000.0,
    );
    let per_chunk =
        summary.chunks <= PER_CHUNK_LINES && summary.chunks * questions.len() <= PER_CHUNK_CELLS;
    for question in &summary.questions {
        text.push_str(&question_lines(summary, question, !per_chunk));
    }
    text.push_str(&failure_lines(summary, runs));
    if per_chunk {
        text.push_str("\nPer chunk:");
        for chunk_id in chunk_ids {
            text.push_str(&chunk_line(summary, questions, runs, chunk_id));
        }
    }
    // R1-8: the batch id stays in the structured result only.
    text.push_str(
        "\nEvery model's answer for every chunk is in this chat's decision history. A tie or a \
         disagreement is unresolved: read those chunks before you report them.",
    );
    text
}

/// One question's lines: the majority and agreement, each model's counts, and
/// (for a batch too big for a line per chunk) the capped ids.
fn question_lines(summary: &VoteSummary, question: &VoteQuestion, list_ids: bool) -> String {
    let rule = match (&question.pass_on, question.threshold) {
        (Some(on), Some(p)) => format!(", yes is P({on}) >= {p:.2}"),
        _ => String::new(),
    };
    let by_model = summary
        .models
        .iter()
        .zip(&question.per_model)
        .map(|(model, counts)| format!("{} {}", model.label, counts_line(counts)))
        .collect::<Vec<_>>()
        .join("; ");
    let mut text = format!(
        "\n- {} ({}{rule}): majority {}; tie {}; all models agreed on {} of {} chunks.\n  by \
         model: {by_model}",
        question.name,
        question.kind.replace('_', "/"),
        counts_line(&question.majority),
        question.ties,
        question.agreed,
        question.compared,
    );
    if !list_ids {
        return text;
    }
    for (answer, ids) in &question.majority_ids {
        let total = question.majority.get(answer).copied().unwrap_or(0);
        let more = total.saturating_sub(ids.len());
        text.push_str(&format!(
            "\n  majority {answer}: {}",
            super::ids_line(ids, more)
        ));
    }
    if !question.disagreements.is_empty() {
        let listed = question
            .disagreements
            .iter()
            .map(|chunk| {
                format!(
                    "{} ({}: {})",
                    chunk.chunk_id,
                    chunk.majority,
                    votes_line(&chunk.votes)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        text.push_str(&format!("\n  disagreed: {listed}"));
        if question.more_disagreements > 0 {
            text.push_str(&format!(" (and {} more)", question.more_disagreements));
        }
    }
    text
}

/// Each model's failures: a model that answered nothing, or its error groups.
fn failure_lines(summary: &VoteSummary, runs: &[ModelRun]) -> String {
    let mut text = String::new();
    for (run, model) in runs.iter().zip(&summary.models) {
        match &run.outcomes {
            Err(why) => text.push_str(&format!("\n{} answered nothing: {why}", model.label)),
            Ok(outcomes) => {
                for group in error_groups(outcomes) {
                    let hit = group.chunk_ids.len() + group.more;
                    text.push_str(&format!(
                        "\n{} failed ({hit} chunk{}): {}: {}",
                        model.label,
                        plural(hit),
                        super::ids_line(&group.chunk_ids, group.more),
                        group.message
                    ));
                }
            }
        }
    }
    text
}

/// One chunk's line: per question, the majority and each model's answer.
fn chunk_line(
    summary: &VoteSummary,
    questions: &IndexMap<String, Question>,
    runs: &[ModelRun],
    chunk_id: &str,
) -> String {
    let parts: Vec<String> = summary
        .questions
        .iter()
        .filter_map(|question| {
            let votes: IndexMap<String, String> = runs
                .iter()
                .zip(&summary.models)
                .filter_map(|(run, model)| {
                    let decision = run
                        .outcomes
                        .as_deref()
                        .ok()?
                        .iter()
                        .find(|o| o.chunk_id == chunk_id)?
                        .decisions
                        .as_ref()?
                        .iter()
                        .find(|d| d.question == question.name)?;
                    Some((
                        model.label.clone(),
                        vote_of(&questions[&question.name], decision),
                    ))
                })
                .collect();
            (!votes.is_empty()).then(|| {
                let majority = majority_of(&votes).unwrap_or_else(|tie| tie);
                format!("{} {majority} ({})", question.name, votes_line(&votes))
            })
        })
        .collect();
    if parts.is_empty() {
        format!("\n- {chunk_id}: no model answered")
    } else {
        format!("\n- {chunk_id}: {}", parts.join("; "))
    }
}
