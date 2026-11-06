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
//!       - 2.0 * saturated                                uses >= 5 and corrected_to == 0
//! ties: updated_at descending, then id
//! ```
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
    let saturated = f64::from(u8::from(entry.uses >= 5 && entry.corrected_to == 0));
    5.0 * context + 3.0 * pinned + 2.0 * corrected + 1.5 * recency + 1.0 * uses
        - 4.0 * collides * (1.0 - context)
        - 2.0 * saturated
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

/// Ranks `eligible` for one request with `context`, and takes the prompt and the fuzzy set
/// from the top 30.
pub fn plan(eligible: &EligibleEntries, context: &str, now: DateTime<Utc>) -> RequestPlan {
    plan_entries(eligible.entries(), context, now)
}

/// [`plan`] over a slice, for this crate's tests. Not public: a ranking is made only from
/// an [`EligibleEntries`], so no caller can rank rows `bias_set` never filtered (DD-36).
pub(crate) fn plan_entries(
    entries: &[RecognitionEntry],
    context: &str,
    now: DateTime<Utc>,
) -> RequestPlan {
    let phrases = context_phrases(clip_context(context));
    let in_context: Vec<bool> = entries.iter().map(|e| in_context(e, &phrases)).collect();
    let scores: Vec<f64> = entries
        .iter()
        .zip(&in_context)
        .map(|(e, c)| score(e, *c, now))
        .collect();
    let mut ranked: Vec<usize> = (0..entries.len()).collect();
    ranked.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        scores[b]
            .total_cmp(&scores[a])
            .then(eb.updated_at.cmp(&ea.updated_at))
            .then(ea.id.cmp(&eb.id))
    });
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
        let saturated = with(entry("EPAS1"), |e| e.uses = 5);
        assert!((score(&saturated, false, now()) - ((6.0f64).ln() / 3.0 - 2.0)).abs() < 1e-9);
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

    #[test]
    fn ties_break_on_updated_at_then_id() {
        let older = with(entry("AAA1"), |e| {
            e.updated_at -= Duration::days(1);
        });
        let newer = entry("BBB1");
        let same_time_b = with(entry("CCC1"), |e| e.id = "dct_b".into());
        let same_time_a = with(entry("DDD1"), |e| e.id = "dct_a".into());
        let entries = vec![older, newer, same_time_b, same_time_a];
        let plan = plan_entries(&entries, "", now());
        let order: Vec<&str> = plan
            .ranked
            .iter()
            .map(|&i| entries[i].written.as_str())
            .collect();
        assert_eq!(order, ["DDD1", "CCC1", "BBB1", "AAA1"]);
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

    #[test]
    fn context_is_clipped_at_a_character_boundary() {
        let long = "é".repeat(3000);
        let clipped = clip_context(&long);
        assert!(clipped.len() <= MAX_CONTEXT_BYTES);
        assert!(clipped.len() >= MAX_CONTEXT_BYTES - 1);
    }
}
