//! Normalized keys (`dictionary.md`, "Step 3: normalized exact keys").
//!
//! A key is what two spellings of one term share once their form is set aside: NFKC,
//! lower case, Greek letters to their names, number words to digits (`zero` to `nineteen`,
//! the tens, `fifty three` and `five three` to `53`), and no spaces, hyphens, periods or
//! apostrophes. So `E-PAS-1`, `EPAS 1` and `E P A S one` all give `epas1`, and `HIF two
//! alpha`, `HIF2-alpha` and `HIF-2α` give `hif2alpha`.
//!
//! The exact key matcher compares a window of the transcript's tokens with the keys of an
//! entry's written form, spoken forms and active misrecognitions; a key says whether any of
//! its digits came from number words, which a `digits_literal` entry never accepts
//! (`PhD two` stays).

use unicode_normalization::UnicodeNormalization;

/// A normalized key and where its digits came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub text: String,
    /// Some digit of `text` came from a number word (`one`, `fifty`).
    pub from_number_words: bool,
}

impl Key {
    /// How many characters the key has: what bounds the window it may match.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

const GREEK: [(char, &str); 24] = [
    ('α', "alpha"),
    ('β', "beta"),
    ('γ', "gamma"),
    ('δ', "delta"),
    ('ε', "epsilon"),
    ('ζ', "zeta"),
    ('η', "eta"),
    ('θ', "theta"),
    ('ι', "iota"),
    ('κ', "kappa"),
    ('λ', "lambda"),
    ('μ', "mu"),
    ('ν', "nu"),
    ('ξ', "xi"),
    ('ο', "omicron"),
    ('π', "pi"),
    ('ρ', "rho"),
    ('σ', "sigma"),
    ('τ', "tau"),
    ('υ', "upsilon"),
    ('φ', "phi"),
    ('χ', "chi"),
    ('ψ', "psi"),
    ('ω', "omega"),
];

const UNITS: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

const TENS: [(&str, u32); 8] = [
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];

fn unit_value(word: &str) -> Option<u32> {
    UNITS.iter().position(|w| *w == word).map(|n| n as u32)
}

fn tens_value(word: &str) -> Option<u32> {
    TENS.iter().find(|(w, _)| *w == word).map(|(_, n)| *n)
}

/// Whether `c` is left out of a key: whitespace, hyphens and dashes, periods, apostrophes.
fn is_dropped(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '-' | '.' | '\'' | '\u{2010}'
                ..='\u{2015}' | '\u{2212}' | '\u{2018}' | '\u{2019}' | '\u{00B7}' | '\u{2027}'
        )
}

/// One token, normalized: NFKC, lower case, Greek letters to names, dropped characters
/// removed.
fn normalize_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for c in token.nfkc().flat_map(char::to_lowercase) {
        if is_dropped(c) {
            continue;
        }
        match GREEK.iter().find(|(letter, _)| *letter == c) {
            Some((_, name)) => out.push_str(name),
            None => out.push(c),
        }
    }
    out
}

/// The key of a window of tokens, in order. A tens word followed by a unit from one to
/// nine is one number (`fifty three` is 53); every other number word is its own digits
/// (`five three` is 53, `nineteen` is 19).
pub fn window_key<S: AsRef<str>>(tokens: &[S]) -> Key {
    let normalized: Vec<String> = tokens.iter().map(|t| normalize_token(t.as_ref())).collect();
    let mut key = Key::default();
    let mut i = 0;
    while let Some(token) = normalized.get(i) {
        if let Some(tens) = tens_value(token) {
            key.from_number_words = true;
            match normalized
                .get(i + 1)
                .and_then(|next| unit_value(next))
                .filter(|unit| (1..=9).contains(unit))
            {
                Some(unit) => {
                    key.text.push_str(&(tens + unit).to_string());
                    i += 2;
                }
                None => {
                    key.text.push_str(&tens.to_string());
                    i += 1;
                }
            }
            continue;
        }
        if let Some(unit) = unit_value(token) {
            key.from_number_words = true;
            key.text.push_str(&unit.to_string());
        } else {
            key.text.push_str(token);
        }
        i += 1;
    }
    key
}

/// The pieces of a form a key reads: split at whitespace and hyphens, so a form keys the
/// same way the transcript's tokens do.
pub fn form_tokens(form: &str) -> Vec<&str> {
    form.split(|c: char| c.is_whitespace() || matches!(c, '-' | '\u{2010}'..='\u{2015}'))
        .filter(|t| !t.is_empty())
        .collect()
}

/// The key of a written form, a spoken form or a misrecognition.
pub fn form_key(form: &str) -> Key {
    window_key(&form_tokens(form))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key table of PR-14's acceptance.
    #[test]
    fn the_key_table() {
        for form in [
            "E-PAS-1",
            "EPAS 1",
            "E P A S one",
            "EPAS1",
            "epas-one",
            "E.P.A.S. 1",
        ] {
            assert_eq!(form_key(form).text, "epas1", "{form}");
        }
        for form in [
            "HIF two alpha",
            "HIF2-alpha",
            "HIF-2α",
            "HIF 2 α",
            "hif2alpha",
        ] {
            assert_eq!(form_key(form).text, "hif2alpha", "{form}");
        }
        assert_eq!(form_key("fifty three").text, "53");
        assert_eq!(form_key("five three").text, "53");
        assert_eq!(form_key("twenty").text, "20");
        assert_eq!(form_key("nineteen").text, "19");
        assert_eq!(form_key("ninety nine").text, "99");
        assert_eq!(
            form_key("twenty ten").text,
            "2010",
            "a tens word takes only 1 to 9"
        );
        assert_eq!(form_key("here's").text, "heres");
        assert_eq!(form_key("Here\u{2019}s").text, "heres");
        assert_eq!(form_key("IGF-1R").text, "igf1r");
        assert_eq!(form_key("P-R-O-K-R2").text, "prokr2");
        assert_eq!(form_key("pH D2").text, "phd2");
        // Full width letters fold under NFKC.
        assert_eq!(form_key("ＥＰＡＳ１").text, "epas1");
    }

    #[test]
    fn a_key_says_where_its_digits_came_from() {
        assert!(form_key("PhD two").from_number_words);
        assert!(!form_key("PHD 2").from_number_words);
        assert!(!form_key("PHD2").from_number_words);
        assert!(form_key("E P A S one").from_number_words);
        assert_eq!(form_key("PhD two").text, form_key("PHD2").text);
        assert_eq!(form_key("EPAS1").len(), 5);
        assert!(form_key("  ").is_empty());
    }
}
