//! The token prompt (`dictionary.md`, "Token prompt", DD-45).
//!
//! 1. Eligible: in the bias set, `bias` not `never`, and not `collides` unless the entry
//!    is in the request's context or pinned.
//! 2. The top 30 by score. When fewer than 30 entries match the context, the remaining
//!    places go to the next entries by score: the prompt is never only the terms predicted
//!    for this utterance, because an oracle narrow prompt cost 18 points on real speech by
//!    deleting the start of the utterance [M].
//! 3. Written forms only, joined with `", "`, no lead in, ascending score so the best entry
//!    is last, then a period ([`with_tail`], PR-14b). Without the period the prompt read
//!    as an unfinished sentence and Whisper dropped the final punctuation of what it wrote
//!    (turbo on 26 of 42 synthetic and 29 of the real clips that had it without a
//!    dictionary); with it, the period came back on 83 of turbo's 88 such clips and all 15
//!    of small.en's, within the rule's recall margins and with no rise in insertions
//!    [M, 2026-11-06]. The period is for the decoder only: correction reads the prompt
//!    without it (`DictionaryRequest::correction_prompt` in the core crate). Spoken forms
//!    and misrecognitions are never prompt content.
//! 4. The worker encodes the list with canonical byte level BPE over the loaded model's
//!    own vocabulary ([`fit`] with `engine::bpe`) and cuts entries from the head until the
//!    prompt is at most 150 tokens; Whisper's window is 223 tokens and drops its head
//!    silently [R].
//! 5. No draft text in v1.

use super::{select::TOP, RecognitionEntry};
use crate::store::BiasPolicy;

/// The separator between the prompt's terms.
pub const SEPARATOR: &str = super::echo::SEPARATOR;

/// What follows the prompt's best term (PR-14b): a period.
pub const TAIL: &str = ".";

/// `terms` (best last) as the worker is sent them: the last term followed by [`TAIL`]
/// when it ends in a letter or a digit, so that the period is its own piece under the
/// GPT-2 pre split and the prompt's canonical ids end with the period token. A last term
/// that ends in other punctuation (`C++`) is sent as it is, because the period would join
/// that punctuation's piece.
pub fn with_tail(mut terms: Vec<String>) -> Vec<String> {
    if let Some(last) = terms.last_mut() {
        if last.chars().next_back().is_some_and(char::is_alphanumeric) {
            last.push_str(TAIL);
        }
    }
    terms
}

/// The prompt's written forms for one request: the top 30 eligible entries of `ranked`
/// (best first), returned in ascending score, best last. Reached through
/// [`super::select::plan`], which takes an [`super::EligibleEntries`].
pub(crate) fn pick(
    entries: &[RecognitionEntry],
    ranked: &[usize],
    in_context: &[bool],
) -> Vec<String> {
    let mut picked: Vec<String> = ranked
        .iter()
        .filter_map(|&i| {
            let entry = entries.get(i)?;
            let context = in_context.get(i).copied().unwrap_or(false);
            let eligible =
                entry.bias != BiasPolicy::Never && (!entry.collides || context || entry.pinned());
            eligible.then(|| entry.written.clone())
        })
        .take(TOP)
        .collect();
    picked.reverse();
    picked
}

/// The prompt's token ids and how many terms from the tail of `terms` (ordered, best last)
/// they hold: the longest tail whose joined text encodes to at most `budget` tokens.
/// `encode` gives a text's canonical token ids, or `None` when it cannot be encoded; a term
/// that cannot be encoded ends the tail there.
pub fn fit<F>(terms: &[String], budget: usize, mut encode: F) -> (Vec<i32>, usize)
where
    F: FnMut(&str) -> Option<Vec<i32>>,
{
    // A first estimate from each term's own cost with its separator, then exact checks of
    // the joined text, dropping head terms until it fits.
    let mut used = 0;
    let mut estimate = 0;
    for term in terms.iter().rev() {
        let Some(cost) = encode(&format!("{SEPARATOR}{term}")).map(|ids| ids.len()) else {
            break;
        };
        // The head term carries no separator, which the estimate counts as one token.
        if estimate + cost > budget + 1 {
            break;
        }
        estimate += cost;
        used += 1;
    }
    let mut joined = |used: usize| -> Option<Vec<i32>> {
        let tail = terms.get(terms.len() - used..)?;
        encode(&tail.join(SEPARATOR)).filter(|ids| ids.len() <= budget)
    };
    // The estimate can be off by a token per term either way: grow while the next term
    // still fits, then shrink until the tail fits.
    let mut best = if used > 0 { joined(used) } else { None };
    while best.is_some() && used < terms.len() {
        match joined(used + 1) {
            Some(ids) => {
                used += 1;
                best = Some(ids);
            }
            None => break,
        }
    }
    while best.is_none() && used > 0 {
        used -= 1;
        best = if used > 0 { joined(used) } else { None };
    }
    match best {
        Some(ids) => (ids, used),
        None => (Vec::new(), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_entries::{entry, with};
    use super::*;

    /// A stand in encoder: one token per character, so lengths are easy to reason about.
    fn chars(text: &str) -> Option<Vec<i32>> {
        Some(text.chars().map(|c| c as i32).collect())
    }

    #[test]
    fn the_prompt_takes_the_top_30_eligible_best_last() {
        let entries = vec![
            entry("EPAS1"),
            entry("SPOKE"),
            with(entry("PHD2"), |e| e.bias = BiasPolicy::Always),
            with(entry("VHL"), |e| e.bias = BiasPolicy::Never),
            entry("UCSF"),
        ];
        // Ranked best first: PHD2, SPOKE, EPAS1, VHL, UCSF.
        let ranked = [2, 1, 0, 3, 4];
        let none = [false; 5];
        assert_eq!(pick(&entries, &ranked, &none), ["UCSF", "EPAS1", "PHD2"]);
        let spoke_in_context = [false, true, false, false, false];
        assert_eq!(
            pick(&entries, &ranked, &spoke_in_context),
            ["UCSF", "EPAS1", "SPOKE", "PHD2"]
        );
        let many: Vec<_> = (0..40).map(|i| entry(&format!("ZQX{i}A"))).collect();
        let ranked: Vec<usize> = (0..40).collect();
        let picked = pick(&many, &ranked, &[false; 40]);
        assert_eq!(picked.len(), 30);
        assert_eq!(
            picked.last().map(String::as_str),
            Some("ZQX0A"),
            "best last"
        );
        assert_eq!(picked.first().map(String::as_str), Some("ZQX29A"));
    }

    /// The tail: a period after the best term, only when that term ends in a letter or a
    /// digit; never on an empty prompt.
    #[test]
    fn the_tail_follows_the_best_term() {
        let terms = |list: &[&str]| -> Vec<String> { list.iter().map(|t| t.to_string()).collect() };
        assert_eq!(
            with_tail(terms(&["UCSF", "EPAS1"])),
            terms(&["UCSF", "EPAS1."])
        );
        assert_eq!(with_tail(terms(&["HIF-2α"])), terms(&["HIF-2α."]));
        assert_eq!(with_tail(terms(&["Advil"])), terms(&["Advil."]));
        assert_eq!(
            with_tail(terms(&["EPAS1", "C++"])),
            terms(&["EPAS1", "C++"])
        );
        assert!(with_tail(Vec::new()).is_empty());
        // `fit` keeps the tail with the best term: it cuts from the head.
        let sent = with_tail(terms(&["AAAA", "BBBB", "CCCC"]));
        let (ids, used) = fit(&sent, 11, chars);
        assert_eq!(used, 2);
        assert_eq!(ids.last(), Some(&('.' as i32)));
    }

    /// With the engine's pre split, the period after a term that ends in a letter or a
    /// digit is its own piece, so its canonical ids end with the period token.
    #[cfg(all(feature = "engine", not(br_no_native)))]
    #[test]
    fn the_tail_is_its_own_piece() {
        use crate::engine::bpe::pre_split;
        for term in ["EPAS1", "Advil", "HIF-2α", "scRNA-seq", "Biorouter"] {
            let sent = with_tail(vec![term.to_string()]);
            let joined = format!("UCSF{SEPARATOR}{}", sent[0]);
            assert_eq!(pre_split(&joined).last(), Some(&"."), "{term}");
        }
    }

    #[test]
    fn fit_cuts_from_the_head() {
        let terms: Vec<String> = ["AAAA", "BBBB", "CCCC"].map(String::from).to_vec();
        // "AAAA, BBBB, CCCC" is 16 characters.
        assert_eq!(fit(&terms, 16, chars).1, 3);
        let (ids, used) = fit(&terms, 15, chars);
        assert_eq!(used, 2);
        assert_eq!(ids.len(), "BBBB, CCCC".len());
        assert_eq!(fit(&terms, 3, chars), (Vec::new(), 0));
        assert_eq!(fit(&[], 150, chars), (Vec::new(), 0));
    }
}
