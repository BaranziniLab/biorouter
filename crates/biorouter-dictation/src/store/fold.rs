//! Fold keys ("Uniqueness, fold keys and twins"): the form two spellings of one term
//! share. A fold key is derived on load and never stored. It drives tombstones and the
//! "same letters" warning, and PR-14's exact key matcher; it never merges two rows,
//! because gene symbols are case significant (human EPAS1, mouse Epas1).

use unicode_normalization::UnicodeNormalization;

/// Greek letter names and their letters, longest first, so a suffix match tries `beta`,
/// `zeta` and `theta` before `eta`, which ends each of them.
const GREEK: [(&str, char); 24] = [
    ("omicron", 'ο'),
    ("epsilon", 'ε'),
    ("upsilon", 'υ'),
    ("lambda", 'λ'),
    ("alpha", 'α'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    ("theta", 'θ'),
    ("kappa", 'κ'),
    ("sigma", 'σ'),
    ("omega", 'ω'),
    ("beta", 'β'),
    ("zeta", 'ζ'),
    ("iota", 'ι'),
    ("eta", 'η'),
    ("rho", 'ρ'),
    ("tau", 'τ'),
    ("phi", 'φ'),
    ("chi", 'χ'),
    ("psi", 'ψ'),
    ("mu", 'μ'),
    ("nu", 'ν'),
    ("xi", 'ξ'),
    ("pi", 'π'),
];

/// Characters a fold key leaves out: whitespace, hyphens and dashes, and dots.
fn is_dropped(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '-' | '.' | '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{00B7}' | '\u{2027}'
        )
}

/// The letter a run of letters spells, when the whole run is a Greek letter's name.
fn whole_name(run: &str) -> Option<char> {
    GREEK
        .iter()
        .find(|(name, _)| run == *name)
        .map(|(_, letter)| *letter)
}

/// A Greek name that ends a run after a capital (`TNFalpha`, `IFNgamma`): the run
/// without the name, and the letter.
fn suffix_name(original: &[char]) -> Option<(usize, char)> {
    let lowered: String = original.iter().collect::<String>().to_lowercase();
    let lowered: Vec<char> = lowered.chars().collect();
    if lowered.len() != original.len() {
        return None;
    }
    GREEK.iter().find_map(|(name, letter)| {
        let n = name.chars().count();
        if original.len() <= n {
            return None;
        }
        let start = original.len() - n;
        let tail: String = lowered.get(start..)?.iter().collect();
        let before = original.get(start - 1)?;
        let first = original.get(start)?;
        (tail == *name && before.is_uppercase() && first.is_lowercase()).then_some((start, *letter))
    })
}

/// The fold key of a written form: NFC, Greek letter names to letters, case folded,
/// with spaces, hyphens and dots removed. `HIF-2alpha`, `HIF2α` and `hif 2 alpha` all
/// fold to `hif2α`; `E-PAS-1` and `EPAS 1` to `epas1`.
///
/// A Greek name is replaced only where it is a whole run of letters (`HIF-2alpha`,
/// `PI3K`) or ends a run right after a capital (`TNFalpha`), never inside an ordinary
/// word, so `Philadelphia` keeps its `phi` and `metabolism` its `eta`.
pub fn fold_key(written: &str) -> String {
    let chars: Vec<char> = written.nfc().collect();
    let mut out = String::with_capacity(written.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !c.is_alphabetic() {
            if !is_dropped(c) {
                out.extend(c.to_lowercase());
            }
            i += 1;
            continue;
        }
        let end = (i..chars.len())
            .find(|&k| !chars[k].is_alphabetic())
            .unwrap_or(chars.len());
        let run = &chars[i..end];
        let lowered = run.iter().collect::<String>().to_lowercase();
        if let Some(letter) = whole_name(&lowered) {
            out.push(letter);
        } else if let Some((at, letter)) = suffix_name(run) {
            for c in &run[..at] {
                out.extend(c.to_lowercase());
            }
            out.push(letter);
        } else {
            out.push_str(&lowered);
        }
        i = end;
    }
    // A final sigma folds to the ordinary one, as a case fold does.
    out.replace('ς', "σ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_term_share_a_key() {
        for (a, b, key) in [
            ("HIF-2alpha", "HIF2α", "hif2α"),
            ("HIF-2 alpha", "hif2Α", "hif2α"),
            ("E-PAS-1", "EPAS 1", "epas1"),
            ("EPAS1", "epas1", "epas1"),
            ("TNFalpha", "TNF-α", "tnfα"),
            ("IFNgamma", "IFN-γ", "ifnγ"),
            ("PI3K", "π3K", "π3k"),
            ("U.C.S.F.", "UCSF", "ucsf"),
            ("Bio\u{2013}router", "Biorouter", "biorouter"),
        ] {
            assert_eq!(fold_key(a), key, "{a}");
            assert_eq!(fold_key(b), key, "{b}");
        }
    }

    #[test]
    fn greek_names_inside_ordinary_words_stay() {
        for (word, key) in [
            ("Philadelphia", "philadelphia"),
            ("metabolism", "metabolism"),
            ("Betaine", "betaine"),
            ("NFkappaB", "nfkappab"),
            ("spine", "spine"),
            ("PHD2", "phd2"),
        ] {
            assert_eq!(fold_key(word), key, "{word}");
        }
    }

    #[test]
    fn case_matters_to_rows_but_not_to_the_key() {
        assert_eq!(fold_key("Epas1"), fold_key("EPAS1"));
        assert_eq!(fold_key("ΣIGMA"), fold_key("σigma"));
        assert_eq!(fold_key("Cafe\u{301}"), fold_key("Caf\u{e9}"));
    }
}
