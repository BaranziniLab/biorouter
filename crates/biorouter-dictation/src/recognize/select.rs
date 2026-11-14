//! Ranking the eligible entries for one request (`dictionary.md`, "Ranking", DD-45).
//!
//! ```text
//! score = 5.0 * in_context
//!       + 3.0 * pinned                                   bias == always
//!       + 2.0 * min(1, corrected_to / 3)
//!       + 1.5 * exp(-ln2 * days_since_last_used / 30)    0 when never used
//!       + 1.0 * min(1, ln(1 + uses) / 3)
//!       + 1.0 * in_active_kb                             0 in v1
//!       - 4.0 * collides * (1 - in_context)
//! ties: updated_at descending, then the fold key, then the written form, then id
//! ```
//!
//! There is no penalty for a term used often and never corrected. One existed (`- 2.0 *
//! saturated`, uses of 5 or more with no correction), meant to give prompt room to terms the
//! model gets wrong. Once `uses` moved (PR-14b) it fired on terms the model writes right only
//! because the prompt carries them: EPAS1 went from 0 of 6 on turbo without the dictionary to
//! 6 of 6 with it, then ranked below every term used fewer than five times after five
//! dictations [M], so in a dictionary of more than 30 terms it would leave the prompt and be
//! misheard again. A use cannot yet tell a prompted dictation from an unprompted one.
//!
//! Terms added together share a score and an `updated_at` second, so the order among
//! them must come from the terms themselves: the fold key (`store::fold::fold_key`) and
//! then the written form, which tells apart two spellings that fold alike (`EPAS1`,
//! `Epas1`). Ids are random, so before PR-14b two dictionaries holding the same terms
//! sent their prompts in different orders and transcribed 3 of 8 clips differently [M].
//! The id now decides only between twins of the two tiers, which `EligibleEntries` has
//! already merged.
//!
//! `in_context` is 1 when a written form, a spoken form or an active misrecognition occurs
//! as a whole word in the request's context, case folded. The context ranks only: it never
//! enters the prompt and is never logged. The usage figures are the ones the entries were
//! built with, from the `public` map for a public rows table and from `all` otherwise
//! (oracle rule 11). Two consumers take a slice of the ranking: the prompt (the top 30)
//! and the fuzzy set (the `match = fuzzy` entries among the top 30).

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::{prompt, EligibleEntries, RecognitionEntry};
use crate::store::fold::fold_key;

/// How many ranked entries the prompt and the fuzzy set draw from.
pub const TOP: usize = 30;
/// The longest context a request may carry, in bytes: 4 KB, the largest of the surfaces'
/// limits (`--context-file`); longer context is cut at a character boundary.
pub const MAX_CONTEXT_BYTES: usize = 4096;
/// The longest form, in words, the context is searched for.
const MAX_FORM_WORDS: usize = 6;

/// What one request uses from its eligible entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestPlan {
    /// Indices into the entries, best first.
    pub ranked: Vec<usize>,
    /// Per entry: some form of it occurs in the context.
    pub in_context: Vec<bool>,
    /// The prompt's written forms, ascending score, best last.
    pub prompt_terms: Vec<String>,
    /// The entries fuzzy matching may reach: the fuzzy capable ones among the top 30.
    pub fuzzy_set: Vec<usize>,
}

/// The words of `text` as the context search reads them: runs of letters and digits,
/// lower case.
fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Every run of 1 to 6 consecutive context words, joined with one space.
fn context_phrases(context: &str) -> HashSet<String> {
    let words = words_of(context);
    let mut phrases = HashSet::with_capacity(words.len() * 2);
    for start in 0..words.len() {
        let mut phrase = String::new();
        for word in words.iter().skip(start).take(MAX_FORM_WORDS) {
            if !phrase.is_empty() {
                phrase.push(' ');
            }
            phrase.push_str(word);
            phrases.insert(phrase.clone());
        }
    }
    phrases
}

/// Whether some form of `entry` occurs as whole words in the context's phrases.
fn in_context(entry: &RecognitionEntry, phrases: &HashSet<String>) -> bool {
    if phrases.is_empty() {
        return false;
    }
    std::iter::once(&entry.written)
        .chain(&entry.spoken_forms)
        .chain(&entry.misrecognitions)
        .any(|form| {
            let words = words_of(form);
            !words.is_empty() && words.len() <= MAX_FORM_WORDS && phrases.contains(&words.join(" "))
        })
}

/// One entry's score.
pub fn score(entry: &RecognitionEntry, in_context: bool, now: DateTime<Utc>) -> f64 {
    let context = f64::from(u8::from(in_context));
    let pinned = f64::from(u8::from(entry.pinned()));
    let corrected = (entry.corrected_to as f64 / 3.0).min(1.0);
    let recency = entry.last_used_at.map_or(0.0, |at| {
        let days = (now - at).num_seconds().max(0) as f64 / 86_400.0;
        (-std::f64::consts::LN_2 * days / 30.0).exp()
    });
    let uses = ((1.0 + entry.uses as f64).ln() / 3.0).min(1.0);
    let collides = f64::from(u8::from(entry.collides));
    5.0 * context + 3.0 * pinned + 2.0 * corrected + 1.5 * recency + 1.0 * uses
        - 4.0 * collides * (1.0 - context)
}

/// Cuts `context` to at most [`MAX_CONTEXT_BYTES`] at a character boundary.
pub fn clip_context(context: &str) -> &str {
    if context.len() <= MAX_CONTEXT_BYTES {
        return context;
    }
    let mut end = MAX_CONTEXT_BYTES;
    while !context.is_char_boundary(end) {
        end -= 1;
    }
    context.get(..end).unwrap_or_default()
}

/// What ranking reads from a set that depends on neither the request nor the usage
/// counters, built once and shared by every request over the same rows (PERF-1): each
/// entry's place in the tie order, and its forms as the context search compares them. A
/// daemon keeps one per unchanged set, so a request ranks with a score per entry and one
/// sort of integers rather than folding 5,000 written forms and comparing strings.
#[derive(Clone, Debug)]
pub struct RankIndex {
    /// Per entry, its place in the tie order: `updated_at` descending, then the fold key,
    /// then the written form, then the id (DD-45).
    tie: Vec<u32>,
    /// Per entry, every form of at most 6 words as lower case words joined with one space.
    phrases: Vec<Vec<String>>,
}

impl RankIndex {
    /// The index of `eligible`'s rows, in their order.
    pub fn new(eligible: &EligibleEntries) -> RankIndex {
        RankIndex::over(eligible.entries())
    }

    pub(crate) fn over(entries: &[RecognitionEntry]) -> RankIndex {
        RankIndex {
            tie: tie_order(entries),
            phrases: entries.iter().map(form_phrases).collect(),
        }
    }

    /// How many entries it indexes.
    pub fn len(&self) -> usize {
        self.tie.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tie.is_empty()
    }
}

/// Each entry's place in DD-45's tie order. A stable sort, so rows equal in every key keep
/// their order, as the ranking's own stable sort kept them.
fn tie_order(entries: &[RecognitionEntry]) -> Vec<u32> {
    let folded: Vec<String> = entries.iter().map(|e| fold_key(&e.written)).collect();
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        eb.updated_at
            .cmp(&ea.updated_at)
            .then(folded[a].cmp(&folded[b]))
            .then(ea.written.cmp(&eb.written))
            .then(ea.id.cmp(&eb.id))
    });
    let mut tie = vec![0u32; entries.len()];
    for (place, &entry) in order.iter().enumerate() {
        tie[entry] = place as u32;
    }
    tie
}

/// An entry's forms as [`in_context`] compares them with the context's phrases.
fn form_phrases(entry: &RecognitionEntry) -> Vec<String> {
    std::iter::once(&entry.written)
        .chain(&entry.spoken_forms)
        .chain(&entry.misrecognitions)
        .filter_map(|form| {
            let words = words_of(form);
            (!words.is_empty() && words.len() <= MAX_FORM_WORDS).then(|| words.join(" "))
        })
        .collect()
}

/// Ranks `eligible` for one request with `context`, and takes the prompt and the fuzzy set
/// from the top 30.
pub fn plan(eligible: &EligibleEntries, context: &str, now: DateTime<Utc>) -> RequestPlan {
    plan_entries(eligible.entries(), context, now)
}

/// [`plan`] with the [`RankIndex`] of `eligible`'s rows, which gives the same plan. An
/// index of another number of rows is not this set's, and the plan is then made without
/// it.
pub fn plan_with(
    eligible: &EligibleEntries,
    index: &RankIndex,
    context: &str,
    now: DateTime<Utc>,
) -> RequestPlan {
    plan_indexed(eligible.entries(), Some(index), context, now)
}

/// [`plan`] over a slice, for this crate's tests. Not public: a ranking is made only from
/// an [`EligibleEntries`], so no caller can rank rows `bias_set` never filtered (DD-36).
pub(crate) fn plan_entries(
    entries: &[RecognitionEntry],
    context: &str,
    now: DateTime<Utc>,
) -> RequestPlan {
    plan_indexed(entries, None, context, now)
}

/// The one ranking: with an index it reads the tie order and the form phrases from it,
/// without one it derives what it needs. Both give the same plan.
pub(crate) fn plan_indexed(
    entries: &[RecognitionEntry],
    index: Option<&RankIndex>,
    context: &str,
    now: DateTime<Utc>,
) -> RequestPlan {
    let index = index.filter(|index| index.len() == entries.len());
    let phrases = context_phrases(clip_context(context));
    let in_context: Vec<bool> = match index {
        Some(index) if !phrases.is_empty() => index
            .phrases
            .iter()
            .map(|forms| forms.iter().any(|form| phrases.contains(form)))
            .collect(),
        _ => entries.iter().map(|e| in_context(e, &phrases)).collect(),
    };
    let scores: Vec<f64> = entries
        .iter()
        .zip(&in_context)
        .map(|(e, c)| score(e, *c, now))
        .collect();
    let built;
    let tie = match index {
        Some(index) => &index.tie,
        None => {
            built = tie_order(entries);
            &built
        }
    };
    // The tie order is a total order over the entries, so this sort needs no stability and
    // gives the order DD-45's comparison gives.
    let mut ranked: Vec<usize> = (0..entries.len()).collect();
    ranked.sort_unstable_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(tie[a].cmp(&tie[b])));
    let prompt_terms = prompt::pick(entries, &ranked, &in_context);
    let fuzzy_set = ranked
        .iter()
        .take(TOP)
        .copied()
        .filter(|&i| entries[i].fuzzy_capable())
        .collect();
    RequestPlan {
        ranked,
        in_context,
        prompt_terms,
        fuzzy_set,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_entries::{entry, with};
    use super::*;
    use crate::store::BiasPolicy;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap_or_default()
    }

    #[test]
    fn the_score_follows_the_formula() {
        let plain = entry("EPAS1");
        assert_eq!(score(&plain, false, now()), 0.0);
        assert_eq!(score(&plain, true, now()), 5.0);
        let pinned = with(entry("EPAS1"), |e| e.bias = BiasPolicy::Always);
        assert_eq!(score(&pinned, false, now()), 3.0);
        let used = with(entry("EPAS1"), |e| {
            e.uses = 4;
            e.corrected_to = 3;
            e.last_used_at = Some(now() - Duration::days(30));
        });
        let expected = 2.0 + 1.5 * 0.5 + (5.0f64).ln() / 3.0;
        assert!((score(&used, false, now()) - expected).abs() < 1e-9);
        // A term used often and never corrected keeps its place: the model may be writing
        // it right only because the prompt carries it.
        let often = with(entry("EPAS1"), |e| e.uses = 5);
        assert!((score(&often, false, now()) - (6.0f64).ln() / 3.0).abs() < 1e-9);
        let rarely = with(entry("EGLN1"), |e| e.uses = 1);
        assert!(score(&often, false, now()) > score(&rarely, false, now()));
        let spoke = entry("SPOKE");
        assert_eq!(score(&spoke, false, now()), -4.0);
        assert_eq!(
            score(&spoke, true, now()),
            5.0,
            "in context, a collision costs nothing"
        );
    }

    #[test]
    fn context_matches_whole_words_and_spoken_forms() {
        let entries = vec![
            entry("EPAS1"),
            with(entry("OMOP"), |e| e.spoken_forms = vec!["oh mop".into()]),
            entry("HIF-2α"),
        ];
        let plan = plan_entries(&entries, "We saw EPAS1. Then: oh MOP, and hif 2α.", now());
        assert_eq!(plan.in_context, [true, true, true]);
        let plan = plan_entries(&entries, "EPAS10 and shopping", now());
        assert_eq!(plan.in_context, [false, false, false], "whole words only");
    }

    /// DD-45's tie rule: `updated_at` descending, then the fold key, then the written
    /// form; the random id never decides between two different terms.
    #[test]
    fn ties_break_on_updated_at_then_the_term() {
        // None of these collides with an English word, so every score is 0.
        let older = with(entry("ZQA1"), |e| {
            e.id = "dct_0".into();
            e.updated_at -= Duration::days(1);
        });
        let entries = vec![
            older,
            with(entry("ZQD1"), |e| e.id = "dct_a".into()),
            with(entry("Epas1"), |e| e.id = "dct_1".into()),
            with(entry("ZQB1"), |e| e.id = "dct_z".into()),
            with(entry("EPAS1"), |e| e.id = "dct_9".into()),
        ];
        assert!(entries.iter().all(|e| score(e, false, now()) == 0.0));
        let plan = plan_entries(&entries, "", now());
        let order: Vec<&str> = plan
            .ranked
            .iter()
            .map(|&i| entries[i].written.as_str())
            .collect();
        // `EPAS1` and `Epas1` fold alike, so their written forms decide; the older `ZQA1`
        // comes last although its fold key and its id come first.
        assert_eq!(order, ["EPAS1", "Epas1", "ZQB1", "ZQD1", "ZQA1"]);
    }

    /// Two dictionaries with the same terms, added in the same second with different
    /// random ids and in a different order, give the same prompt (PR-14's live check
    /// found 3 of 8 clips transcribed differently before).
    #[test]
    fn the_same_terms_give_the_same_prompt() {
        let terms = [
            "EPAS1",
            "EGLN1",
            "HIF-2α",
            "UCSF",
            "Baranzini",
            "OMOP",
            "natalizumab",
            "ocrelizumab",
            "scRNA-seq",
            "Biorouter",
            "VHL",
        ];
        let dictionary = |ids: &dyn Fn(usize) -> String, reversed: bool| {
            let mut entries: Vec<RecognitionEntry> = terms
                .iter()
                .enumerate()
                .map(|(i, t)| with(entry(t), |e| e.id = ids(i)))
                .collect();
            if reversed {
                entries.reverse();
            }
            plan_entries(&entries, "", now()).prompt_terms
        };
        let first = dictionary(&|i| format!("dct_{:02}", i), false);
        let second = dictionary(&|i| format!("dct_{:02}", 99 - i), true);
        assert_eq!(first, second);
        // Best last: the earliest fold key.
        assert_eq!(first.last().map(String::as_str), Some("Baranzini"));
    }

    #[test]
    fn the_fuzzy_set_is_the_fuzzy_capable_top_30() {
        let mut entries: Vec<RecognitionEntry> = (0..40u8)
            .map(|i| {
                let name = format!(
                    "Zorvatin{}{}",
                    char::from(b'a' + i / 26),
                    char::from(b'a' + i % 26)
                );
                with(entry(&name), |e| {
                    e.uses = 40 - u64::from(i);
                    e.corrected_to = 1;
                })
            })
            .collect();
        entries.push(entry("EPAS1"));
        let plan = plan_entries(&entries, "", now());
        assert_eq!(plan.fuzzy_set.len(), TOP);
        assert!(plan.fuzzy_set.iter().all(|&i| entries[i].fuzzy_capable()));
        assert!(!plan.fuzzy_set.contains(&40), "EPAS1 is exact");
    }

    /// The ranking PR-14b shipped, kept here as the reference: one stable sort by the score,
    /// then DD-45's tie keys compared as strings.
    fn reference_ranked(entries: &[RecognitionEntry], context: &str) -> Vec<usize> {
        let phrases = context_phrases(clip_context(context));
        let in_context: Vec<bool> = entries.iter().map(|e| in_context(e, &phrases)).collect();
        let scores: Vec<f64> = entries
            .iter()
            .zip(&in_context)
            .map(|(e, c)| score(e, *c, now()))
            .collect();
        let folded: Vec<String> = entries.iter().map(|e| fold_key(&e.written)).collect();
        let mut ranked: Vec<usize> = (0..entries.len()).collect();
        ranked.sort_by(|&a, &b| {
            let (ea, eb) = (&entries[a], &entries[b]);
            scores[b]
                .total_cmp(&scores[a])
                .then(eb.updated_at.cmp(&ea.updated_at))
                .then(folded[a].cmp(&folded[b]))
                .then(ea.written.cmp(&eb.written))
                .then(ea.id.cmp(&eb.id))
        });
        ranked
    }

    /// PERF-1: a plan made with a [`RankIndex`] is the plan made without one, and both rank
    /// as the reference does, over ties, usage, collisions, pins, spoken forms and
    /// misrecognitions, with and without context; an index of other rows is not used.
    #[test]
    fn the_index_gives_the_plan_made_without_it() {
        let names = [
            "EPAS1",
            "Epas1",
            "SPOKE",
            "EGLN1",
            "UCSF",
            "Baranzini",
            "OMOP",
            "VHL",
            "ZQA1",
            "natalizumab",
            "CRISPRi",
            "Biorouter",
            "scRNA-seq",
            "HIF-2α",
            "PHD2",
            "Modulite",
        ];
        let mut entries: Vec<RecognitionEntry> = Vec::new();
        for round in 0..6u8 {
            for (i, name) in names.iter().enumerate() {
                let i = i as u8;
                entries.push(with(
                    entry(&format!("{name}{}", "x".repeat(round.into()))),
                    |e| {
                        e.id = format!("dct_{round}_{i}");
                        // Three groups share an `updated_at` second, so the tie keys decide.
                        e.updated_at -= Duration::days(i64::from((i + round) % 3));
                        if (i + round).is_multiple_of(4) {
                            e.uses = u64::from(i);
                            e.last_used_at = Some(now() - Duration::days(i64::from(i)));
                        }
                        if (i + round).is_multiple_of(5) {
                            e.corrected_to = 1;
                        }
                        if (i + round).is_multiple_of(7) {
                            e.bias = BiasPolicy::Always;
                        }
                        if (i + round).is_multiple_of(6) {
                            e.bias = BiasPolicy::Never;
                        }
                        if i.is_multiple_of(3) {
                            e.spoken_forms = vec![format!("spoken {i} form {round}")];
                        }
                        if i % 4 == 1 {
                            e.misrecognitions = vec![format!("heard {i} wrong {round}")];
                        }
                    },
                ));
            }
        }
        let index = RankIndex::over(&entries);
        for context in [
            "",
            "We saw EPAS1 and spoken 3 form 2, then SPOKE; heard 5 wrong 0 and UCSFx.",
            "baranzini xx and omop xxx with vhl",
        ] {
            let reference = reference_ranked(&entries, context);
            let without = plan_entries(&entries, context, now());
            let indexed = plan_indexed(&entries, Some(&index), context, now());
            assert_eq!(without.ranked, reference, "{context:?}");
            assert_eq!(indexed, without, "{context:?}");
        }
        assert!(
            plan_entries(&entries, names.join(" ").as_str(), now())
                .in_context
                .iter()
                .any(|c| *c),
            "the fixture puts some entries in context"
        );
        let other = RankIndex::over(&entries[1..]);
        assert_eq!(
            plan_indexed(&entries, Some(&other), "EPAS1", now()),
            plan_entries(&entries, "EPAS1", now()),
            "an index of other rows is left out"
        );
    }

    #[test]
    fn context_is_clipped_at_a_character_boundary() {
        let long = "é".repeat(3000);
        let clipped = clip_context(&long);
        assert!(clipped.len() <= MAX_CONTEXT_BYTES);
        assert!(clipped.len() >= MAX_CONTEXT_BYTES - 1);
    }
}
