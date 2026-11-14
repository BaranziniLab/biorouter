//! Echo, trailing recital and collapse (DD-48, `dictionary.md`, "Echo, collapse and one
//! retry").
//!
//! The echo checks read the output against the exact prompt that was sent, never against
//! the dictionary: the terms the worker fitted into the token budget (the tail of the
//! ranked list, best last), in their order. Their thresholds are OpenWhispr's design
//! constants [R], not measurements of Whisper:
//!
//! | Check | Rule |
//! |---|---|
//! | Whole echo | 90 % or more of the output's unique words are prompt words, and the output keeps the prompt's shape: a word repeated 3 or more times, a fragment of at most 30 characters that ends on the separator, or 3 or more consecutive entries in prompt order |
//! | Trailing recital | the last sentence copies 2 or more consecutive entries from the head of the prompt, or 5 or more that end at its tail |
//! | Collapse or deleted start | fewer than 1.0 output words per second over more than 3 s of detected speech |
//!
//! A whole echo or a collapse earns one retry with no prompt, the detector and the
//! decoder bonus still on, and the retry is adopted only when it is not itself prompt
//! like. A trailing recital is stripped and not retried. A retry is made only when a
//! prompt was sent: it differs from the first decode only by the prompt, so without one
//! it would decode the same audio the same way and cost a second decode for nothing.
//! Until PR-14 no prompt is sent, so every check runs and no retry is ever made.

use std::collections::{HashMap, HashSet};

use super::{text, RetryReason};

/// The share of the output's unique words that must be prompt words for a whole echo.
pub const ECHO_SHARE: f64 = 0.9;
/// A word repeated this often keeps the prompt's shape.
pub const ECHO_REPEATS: usize = 3;
/// A fragment of at most this many characters ending on the separator keeps it too.
pub const ECHO_FRAGMENT_CHARS: usize = 30;
/// So do this many consecutive entries in prompt order.
pub const ECHO_RUN: usize = 3;
/// A recital from the head of the prompt: at least this many consecutive entries.
pub const RECITAL_HEAD: usize = 2;
/// A recital ending at the tail of the prompt: at least this many.
pub const RECITAL_TAIL: usize = 5;
/// Fewer output words per second of detected speech than this is a collapse.
pub const COLLAPSE_WORDS_PER_SECOND: f64 = 1.0;
/// Only over more detected speech than this.
pub const COLLAPSE_MIN_SPEECH_MS: u64 = 3_000;
/// What joins the prompt's terms (`dictionary.md`, "The token prompt").
pub const SEPARATOR: &str = ", ";

/// What one decode's output is worth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assessment {
    /// The output, trimmed, with a trailing recital removed.
    pub text: String,
    pub recital_stripped: bool,
    /// Set when the output is a whole echo or a collapse. Act on it only when
    /// [`retry_changes_anything`] says so.
    pub retry: Option<RetryReason>,
}

/// Reads one decode's `output`, given the prompt terms that were sent and the
/// milliseconds of speech the voice detector found in the decoded audio.
pub fn assess(output: &str, prompt: &[String], speech_ms: u64) -> Assessment {
    if is_whole_echo(output, prompt) {
        return Assessment {
            text: output.trim().to_string(),
            recital_stripped: false,
            retry: Some(RetryReason::Echo),
        };
    }
    // The collapse is judged on the output as written: a recital adds words, so it can
    // only hide a collapse, never make one.
    let retry = is_collapse(output, speech_ms).then_some(RetryReason::Collapse);
    match strip_trailing_recital(output, prompt) {
        Some(text) => Assessment {
            text,
            recital_stripped: true,
            retry,
        },
        None => Assessment {
            text: output.trim().to_string(),
            recital_stripped: false,
            retry,
        },
    }
}

/// Whether a retry can give another answer. It drops the prompt and keeps everything
/// else, so with no prompt sent it would repeat the same greedy decode.
pub fn retry_changes_anything(prompt_sent: bool) -> bool {
    prompt_sent
}

/// The retry's output is used only when it is not itself prompt like; otherwise the
/// transcript reports no speech.
pub fn adopt_retry(retry_output: &str, prompt: &[String]) -> bool {
    !is_whole_echo(retry_output, prompt) && strip_trailing_recital(retry_output, prompt).is_none()
}

/// The words of `text`, case folded: runs of letters and digits, so punctuation and the
/// separator never count as words.
fn folded_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Each prompt entry as its words. Entries with no word are left out.
fn entries(prompt: &[String]) -> Vec<Vec<String>> {
    prompt
        .iter()
        .map(|term| folded_words(term))
        .filter(|words| !words.is_empty())
        .collect()
}

/// Every maximal run of consecutive prompt entries that appears in `words` in prompt
/// order, each entry's words directly after the last's: `(first entry, length)`.
fn runs(words: &[String], entries: &[Vec<String>]) -> Vec<(usize, usize)> {
    let at = |position: usize, entry: &Vec<String>| -> bool {
        words
            .get(position..position + entry.len())
            .is_some_and(|slice| slice == entry.as_slice())
    };
    let mut found = Vec::new();
    for start in 0..words.len() {
        for (first, entry) in entries.iter().enumerate() {
            if !at(start, entry) {
                continue;
            }
            let (mut length, mut position) = (1, start + entry.len());
            while let Some(next) = entries.get(first + length) {
                if !at(position, next) {
                    break;
                }
                position += next.len();
                length += 1;
            }
            found.push((first, length));
        }
    }
    found
}

/// The whole echo rule.
pub fn is_whole_echo(output: &str, prompt: &[String]) -> bool {
    let entries = entries(prompt);
    let words = folded_words(output);
    if entries.is_empty() || words.is_empty() {
        return false;
    }
    let prompt_words: HashSet<&String> = entries.iter().flatten().collect();
    let unique: HashSet<&String> = words.iter().collect();
    let shared = unique.iter().filter(|w| prompt_words.contains(**w)).count();
    if (shared as f64) < ECHO_SHARE * unique.len() as f64 {
        return false;
    }
    let mut counts: HashMap<&String, usize> = HashMap::new();
    for word in &words {
        *counts.entry(word).or_default() += 1;
    }
    let repeated = counts.values().any(|n| *n >= ECHO_REPEATS);
    let trimmed = output.trim();
    let fragment =
        trimmed.chars().count() <= ECHO_FRAGMENT_CHARS && trimmed.ends_with(SEPARATOR.trim_end());
    let in_order = runs(&words, &entries)
        .iter()
        .any(|(_, length)| *length >= ECHO_RUN);
    repeated || fragment || in_order
}

/// `output` split before its last sentence: everything up to and including the last
/// sentence end that has more text after it, and that text.
fn split_last_sentence(output: &str) -> (&str, &str) {
    let trimmed = output.trim_end();
    let mut cut = 0;
    let mut chars = trimmed.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?')
            && chars.peek().is_some_and(|(_, next)| next.is_whitespace())
        {
            cut = i + c.len_utf8();
        }
    }
    (
        trimmed.get(..cut).unwrap_or_default(),
        trimmed.get(cut..).unwrap_or_default(),
    )
}

/// The output without its last sentence when that sentence recites the prompt; `None`
/// when it does not.
pub fn strip_trailing_recital(output: &str, prompt: &[String]) -> Option<String> {
    let entries = entries(prompt);
    if entries.is_empty() {
        return None;
    }
    let (head, last) = split_last_sentence(output);
    let words = folded_words(last);
    let recital = runs(&words, &entries).iter().any(|&(first, length)| {
        (first == 0 && length >= RECITAL_HEAD)
            || (first + length == entries.len() && length >= RECITAL_TAIL)
    });
    recital.then(|| head.trim().to_string())
}

/// The collapse rule: fewer than one word per second over more than 3 s of speech.
pub fn is_collapse(output: &str, speech_ms: u64) -> bool {
    speech_ms > COLLAPSE_MIN_SPEECH_MS
        && (text::word_count(output) as f64)
            < COLLAPSE_WORDS_PER_SECOND * speech_ms as f64 / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(terms: &[&str]) -> Vec<String> {
        terms.iter().map(|t| t.to_string()).collect()
    }

    /// Lowest ranked first, best last, as the worker sends them.
    fn genes() -> Vec<String> {
        prompt(&[
            "EGLN1", "EPAS1", "HIF2A", "VHL", "PHD2", "ventolin", "clenil",
        ])
    }

    #[test]
    fn a_whole_echo_needs_prompt_words_and_the_prompts_shape() {
        let p = genes();
        // 3 or more consecutive entries in prompt order.
        assert!(is_whole_echo("EPAS1, HIF2A, VHL", &p));
        assert!(is_whole_echo("EPAS1 HIF2A VHL PHD2.", &p));
        // A word repeated 3 or more times.
        assert!(is_whole_echo("ventolin ventolin ventolin", &p));
        // A short fragment that ends on the separator.
        assert!(is_whole_echo("EPAS1, HIF2A,", &p));
        // Prompt words, but not the prompt's shape.
        assert!(!is_whole_echo("HIF2A and EPAS1", &p));
        assert!(!is_whole_echo("VHL, EPAS1", &p));
        // The prompt's shape, but under 90 % prompt words.
        assert!(!is_whole_echo(
            "patients with EPAS1, HIF2A, VHL variants were seen today in clinic",
            &p
        ));
        // Real speech with one term.
        assert!(!is_whole_echo("Take the ventolin twice a day.", &p));
        // No prompt, no echo.
        assert!(!is_whole_echo("EPAS1, HIF2A, VHL", &[]));
        assert!(!is_whole_echo("", &p));
    }

    #[test]
    fn a_trailing_recital_is_stripped_and_nothing_else_is() {
        let p = genes();
        // Two entries from the head of the prompt, in order.
        assert_eq!(
            strip_trailing_recital("Start the inhaler today. EGLN1, EPAS1.", &p).as_deref(),
            Some("Start the inhaler today.")
        );
        // Five entries ending at the tail.
        assert_eq!(
            strip_trailing_recital(
                "Two puffs as needed. VHL, PHD2, ventolin, clenil and HIF2A? HIF2A, VHL, PHD2, ventolin, clenil",
                &p
            )
            .as_deref(),
            Some("Two puffs as needed. VHL, PHD2, ventolin, clenil and HIF2A?")
        );
        // Four tail entries are not a recital, nor are two from the middle.
        assert_eq!(
            strip_trailing_recital("Fine. VHL, PHD2, ventolin, clenil", &p),
            None
        );
        assert_eq!(strip_trailing_recital("Fine. HIF2A, VHL", &p), None);
        // A recital only counts in the last sentence.
        assert_eq!(
            strip_trailing_recital("EGLN1, EPAS1. Then we stop.", &p),
            None
        );
        assert_eq!(strip_trailing_recital("EGLN1, EPAS1.", &[]), None);
        // The whole output can be the recital.
        assert_eq!(
            strip_trailing_recital("EGLN1, EPAS1, HIF2A", &p).as_deref(),
            Some("")
        );
    }

    #[test]
    fn a_collapse_is_under_one_word_per_second_over_three_seconds() {
        assert!(is_collapse("", 3_001));
        assert!(is_collapse("one two three", 4_000));
        assert!(!is_collapse("one two three four", 4_000));
        // Three seconds or less is never judged.
        assert!(!is_collapse("", 3_000));
        assert!(!is_collapse("", 0));
        // Punctuation is not a word.
        assert!(is_collapse("- ... -- one", 2_000 + 2_000));
    }

    #[test]
    fn assess_decides_one_retry_and_strips_a_recital() {
        let p = genes();
        let echo = assess("EPAS1, HIF2A, VHL", &p, 6_000);
        assert_eq!(echo.retry, Some(RetryReason::Echo));
        let collapse = assess("Okay.", &p, 8_000);
        assert_eq!(collapse.retry, Some(RetryReason::Collapse));
        assert_eq!(collapse.text, "Okay.");
        let recital = assess(
            "Start the inhaler today, two puffs, then review. EGLN1, EPAS1.",
            &p,
            5_000,
        );
        assert_eq!(recital.retry, None, "a recital is stripped, not retried");
        assert!(recital.recital_stripped);
        assert_eq!(
            recital.text,
            "Start the inhaler today, two puffs, then review."
        );
        let clean = assess(" Take the ventolin twice a day. ", &p, 2_500);
        assert_eq!(
            clean,
            Assessment {
                text: "Take the ventolin twice a day.".into(),
                recital_stripped: false,
                retry: None,
            }
        );
    }

    /// Without a prompt only the collapse can fire, and the retry it asks for is never
    /// made: it would repeat the same decode.
    #[test]
    fn with_no_prompt_nothing_is_retried() {
        let assessment = assess("Okay.", &[], 9_000);
        assert_eq!(assessment.retry, Some(RetryReason::Collapse));
        assert!(!retry_changes_anything(false));
        assert!(retry_changes_anything(true));
        assert_eq!(assess("EPAS1, HIF2A, VHL", &[], 3_000).retry, None);
    }

    #[test]
    fn a_retry_is_adopted_only_when_it_is_not_prompt_like() {
        let p = genes();
        assert!(adopt_retry("Take two puffs of ventolin.", &p));
        assert!(adopt_retry("", &p), "an empty retry is not prompt like");
        assert!(!adopt_retry("EPAS1, HIF2A, VHL, PHD2", &p));
        assert!(!adopt_retry("Take two puffs. EGLN1, EPAS1.", &p));
    }

    #[test]
    fn prompt_entries_with_several_words_match_word_by_word() {
        let p = prompt(&["bio router", "HIF-2 alpha", "C++", "Lemsip"]);
        // Two entries in order, 32 characters: not the prompt's shape.
        assert!(!is_whole_echo("Bio Router, HIF-2 alpha, Lemsip", &p));
        // "C++" folds to the word "c"; four entries in order are.
        assert!(is_whole_echo("bio router HIF 2 alpha c lemsip", &p));
    }
}
