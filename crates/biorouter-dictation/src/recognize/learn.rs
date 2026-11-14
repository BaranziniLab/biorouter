//! Checks for short corrections made to dictated prose (DD-58).
//!
//! The caller applies its secret and PII scans before the pair reaches the store.
//! No transcript or correction history is kept here.

use super::{correct::double_metaphone, key, words};
use crate::store::validate;

pub const MAX_PAIRS_PER_MESSAGE: usize = 3;
pub const MAX_PAIR_WORDS: usize = 3;

const FUNCTION_WORDS: &str = "a an the and or but for nor so yet of to in on at by from with as is it be am are was were been being this that these those i me my we us our you your he him his she her they them their not no yes do does did have has had can could will would shall should may might must than then there here who what when where why how all any each every some one two only also very just gonna wanna gotta";

fn function_word(text: &str) -> bool {
    FUNCTION_WORDS.split_whitespace().any(|word| word == text)
}

/// The format shared by corrections and rejections. The latter must still be
/// admitted when the person undoes a correction of an ordinary English word.
pub fn pair_fits(from: &str, to: &str) -> bool {
    from != to
        && !from.contains(',')
        && [from, to].into_iter().all(|text| {
            !text.contains(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
                && validate::term(text).is_ok()
                && text.split_whitespace().count() <= MAX_PAIR_WORDS
                && !identifier(text)
        })
}

fn identifier(text: &str) -> bool {
    let mut digits = 0;
    for c in text.chars() {
        digits = if c.is_ascii_digit() { digits + 1 } else { 0 };
        if digits >= 5 {
            return true;
        }
    }
    false
}

fn inflection(from: &str, to: &str) -> bool {
    let from = from.to_lowercase();
    let to = to.to_lowercase();
    if from.contains(['\'', '\u{2019}']) || to.contains(['\'', '\u{2019}']) {
        return true;
    }
    let matches_suffix = [(from.as_str(), to.as_str()), (to.as_str(), from.as_str())]
        .into_iter()
        .any(|(stem, word)| {
            word.strip_prefix(stem).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "s" | "es" | "ed" | "d" | "ing" | "ly" | "er" | "est"
                )
            })
        });
    matches_suffix
}

fn term_like(text: &str) -> bool {
    text.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        let capitals = word.chars().filter(|c| c.is_uppercase()).count();
        let folded = word.to_lowercase();
        !function_word(&folded)
            && (word.chars().skip(1).any(|c| c.is_uppercase())
                || (letters > 0 && word.chars().any(|c| c.is_ascii_digit()))
                || (letters >= 2 && capitals == letters)
                || (letters >= 3 && !words::is_common(word)))
    })
}

/// Whether this pair can teach a term. This is the authority behind the
/// renderer and terminal learners; callers still scan both sides for secrets
/// and PII through the core crate before writing.
pub fn accepts_correction(from: &str, to: &str) -> bool {
    if !pair_fits(from, to) || !term_like(to) || inflection(from, to) {
        return false;
    }
    let folded = from.to_lowercase();
    if !from.contains(char::is_whitespace) && function_word(&folded) {
        return false;
    }
    let source = key::form_key(from).text;
    let target = key::form_key(to).text;
    if source.is_empty() || target.is_empty() {
        return false;
    }
    if source == target || strsim::normalized_levenshtein(&source, &target) >= 0.35 {
        return true;
    }
    let source_sound = double_metaphone(&source);
    let target_sound = double_metaphone(&target);
    let sounds_match = [source_sound.0.as_str(), source_sound.1.as_str()]
        .into_iter()
        .filter(|code| !code.is_empty())
        .any(|left| {
            [target_sound.0.as_str(), target_sound.1.as_str()]
                .into_iter()
                .any(|right| !right.is_empty() && strsim::levenshtein(left, right) <= 1)
        });
    sounds_match
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        from: String,
        to: String,
        accepted: bool,
    }

    #[test]
    fn golden_pairs() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("learn_cases.json")).unwrap();
        for case in cases {
            assert_eq!(
                accepts_correction(&case.from, &case.to),
                case.accepted,
                "{} -> {}",
                case.from,
                case.to
            );
        }
    }

    #[test]
    fn limits_and_rejections() {
        assert!(!pair_fits("one two three four", "EPAS1"));
        assert!(!pair_fits("a pass, one", "EPAS1"));
        assert!(!pair_fits("Epas one", &"X".repeat(65)));
        assert!(!pair_fits("Epas\none", "EPAS1"));
        assert!(!pair_fits("Epas one", "EPAS1\n"));
        assert!(pair_fits("spoke", "SPOKE"));
        assert!(!accepts_correction("genes", "Gene"));
    }
}
