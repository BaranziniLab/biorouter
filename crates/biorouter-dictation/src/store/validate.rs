//! The one validator ("Validation"). The routes, the capability, the extractor, the
//! learner, the CLI and import all call it, so a term refused on one surface is refused
//! on every other.
//!
//! The leaf crate cannot call the core's secret scanner, so every caller that can also
//! refuses a term for which `biorouter::guardrails::secret_output::redact_text` answers
//! `Some`, as [`ValidationCode::SecretShaped`].
//!
//! `common_word_misrecognition` needs the English word list, which PR-14 adds with the
//! recognizer; until then no writer stores a misrecognition from an agent, the extractor
//! or the learner, the only rows that rule covers.

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

/// The longest written form, spoken form or misrecognition, in characters after NFC.
pub const MAX_TERM_CHARS: usize = 64;
/// The longest context cue.
pub const MAX_CUE_CHARS: usize = 32;
/// The most whitespace separated words in a term.
pub const MAX_WORDS: usize = 6;
pub const MAX_SPOKEN_FORMS: usize = 8;
pub const MAX_MISRECOGNITIONS: usize = 16;
pub const MAX_CONTEXT_CUES: usize = 8;
/// A note is one line of at most this many characters.
pub const MAX_NOTE_CHARS: usize = 280;
/// An agent's reason is one line of at most this many characters.
pub const MAX_REASON_CHARS: usize = 140;

/// Why a term was refused. The wire code is the snake_case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCode {
    Empty,
    TooLong,
    TooManyWords,
    LineBreak,
    ControlCharacter,
    AngleBracket,
    Url,
    Email,
    Path,
    DigitsOnly,
    LongHex,
    SecretShaped,
    /// More spoken forms, misrecognitions or cues than a row holds.
    TooManyForms,
}

impl ValidationCode {
    pub const ALL: [Self; 13] = [
        Self::Empty,
        Self::TooLong,
        Self::TooManyWords,
        Self::LineBreak,
        Self::ControlCharacter,
        Self::AngleBracket,
        Self::Url,
        Self::Email,
        Self::Path,
        Self::DigitsOnly,
        Self::LongHex,
        Self::SecretShaped,
        Self::TooManyForms,
    ];

    /// The code a wire name names.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == code)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::TooManyWords => "too_many_words",
            Self::LineBreak => "line_break",
            Self::ControlCharacter => "control_character",
            Self::AngleBracket => "angle_bracket",
            Self::Url => "url",
            Self::Email => "email",
            Self::Path => "path",
            Self::DigitsOnly => "digits_only",
            Self::LongHex => "long_hex",
            Self::SecretShaped => "secret_shaped",
            Self::TooManyForms => "too_many_forms",
        }
    }

    /// The sentence a person sees ("Validation and cap texts"). `empty` has no catalog
    /// entry of its own and reads as MSG-TERM-PLAIN.
    pub fn message(self) -> &'static str {
        match self {
            Self::TooLong => MSG_TERM_TOO_LONG,
            Self::TooManyWords => MSG_TERM_WORDS,
            Self::Empty | Self::LineBreak | Self::ControlCharacter | Self::AngleBracket => {
                MSG_TERM_PLAIN
            }
            Self::Url | Self::Email | Self::Path => MSG_TERM_ADDRESS,
            Self::DigitsOnly | Self::LongHex | Self::SecretShaped => MSG_TERM_IDENTIFIER,
            Self::TooManyForms => MSG_TERM_FORMS,
        }
    }
}

impl std::fmt::Display for ValidationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// MSG-TERM-TOO-LONG.
pub const MSG_TERM_TOO_LONG: &str = "Use a term of at most 64 characters.";
/// MSG-TERM-WORDS.
pub const MSG_TERM_WORDS: &str = "Use a term of at most six words.";
/// MSG-TERM-PLAIN.
pub const MSG_TERM_PLAIN: &str = "Use one line of plain text.";
/// MSG-TERM-ADDRESS.
pub const MSG_TERM_ADDRESS: &str =
    "A dictionary term cannot be a web address, an email address or a file path.";
/// MSG-TERM-IDENTIFIER.
pub const MSG_TERM_IDENTIFIER: &str =
    "This looks like an identifier or a key, so it was not added.";
/// MSG-TERM-FORMS: `too_many_forms`.
pub const MSG_TERM_FORMS: &str =
    "Use at most eight spoken forms and sixteen misrecognitions for a term.";

/// Unicode formatting characters a term may not hide: zero width characters, the
/// direction overrides and isolates, the word joiner range and the byte order mark.
/// `char::is_control` covers only the C0 and C1 controls, and these reorder or hide text
/// as surely as they do.
fn is_formatting(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// NFC, then trimmed.
pub fn normalize(raw: &str) -> String {
    raw.nfc().collect::<String>().trim().to_string()
}

/// Every character that ends a line: CR and LF, NEL, and the Unicode line and paragraph
/// separators. The last two are not control characters (`char::is_control` misses them),
/// and `trim` removes them only at the ends.
const LINE_BREAKS: [char; 5] = ['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'];

/// The checks every one line text field shares: no line break, no control or
/// formatting character, not empty, at most `max_chars`.
fn plain_line(text: &str, max_chars: usize) -> Result<(), ValidationCode> {
    if text.contains(LINE_BREAKS) {
        return Err(ValidationCode::LineBreak);
    }
    if text.chars().any(|c| c.is_control() || is_formatting(c)) {
        return Err(ValidationCode::ControlCharacter);
    }
    let length = text.chars().count();
    if length == 0 {
        return Err(ValidationCode::Empty);
    }
    if length > max_chars {
        return Err(ValidationCode::TooLong);
    }
    Ok(())
}

fn is_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    let Some((host, tld)) = domain.rsplit_once('.') else {
        return false;
    };
    !local.is_empty()
        && !host.is_empty()
        && !domain.contains('@')
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
}

fn is_path(text: &str) -> bool {
    let mut chars = text.chars();
    let drive = matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(letter), Some(':'), Some('\\' | '/')) if letter.is_ascii_alphabetic()
    );
    drive
        || text.starts_with(['/', '~'])
        || text.starts_with("./")
        || text.starts_with("../")
        || text.starts_with(".\\")
        || text.starts_with("..\\")
        || text.chars().filter(|c| matches!(c, '/' | '\\')).count() >= 2
}

fn is_digits_only(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || "-._,:/+()#%".contains(c))
}

fn has_long_hex(text: &str) -> bool {
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_hexdigit() {
            run += 1;
            if run >= 16 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// One piece of a hyphenated phrase: a word (`Factor`, `alpha`, `RNA`) of at most 16
/// letters, a number of at most 6 digits, or a symbol of at most 8 capitals and digits
/// (`HIF1A`, `1A`). The pieces of a random key mix cases and digits within one piece,
/// and any `_`, `+`, `/` or `=` makes a piece something else.
fn is_phrase_piece(piece: &str) -> bool {
    let length = piece.chars().count();
    if piece.chars().all(|c| c.is_ascii_digit()) {
        return length <= 6;
    }
    if piece.chars().all(|c| c.is_ascii_alphabetic()) {
        let capitalized = piece.chars().skip(1).all(|c| c.is_ascii_lowercase());
        let capitals = piece.chars().all(|c| c.is_ascii_uppercase());
        return length <= 16 && (capitalized || capitals);
    }
    length <= 8
        && piece
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// A known key prefix on a token of key length, or a base64 shaped run of 32 or more
/// characters that mixes digits with upper and lower case letters. The mix keeps a long
/// hyphenated phrase with no digit (`Chronic-Obstructive-Pulmonary-Disease`) out; a real
/// key carries all three. A run that holds a hyphen and whose hyphen separated pieces are
/// each a word, a number or a short symbol is a phrase even with a digit
/// (`Hypoxia-Inducible-Factor-1-alpha`); the pieces of a random key are never all of
/// those shapes.
fn is_secret_shaped(text: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "sk-",
        "AKIA",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "xox",
    ];
    let prefixed = text.split_whitespace().any(|token| {
        token.chars().count() >= 12 && PREFIXES.iter().any(|prefix| token.starts_with(prefix))
    });
    if prefixed {
        return true;
    }
    let base64 = |c: char| c.is_ascii_alphanumeric() || "+/=_-".contains(c);
    let mut run: Vec<char> = Vec::new();
    let check = |run: &mut Vec<char>| {
        let mixed = run.len() >= 32
            && run.iter().any(|c| c.is_ascii_digit())
            && run.iter().any(|c| c.is_ascii_uppercase())
            && run.iter().any(|c| c.is_ascii_lowercase());
        let shaped = mixed && {
            let text: String = run.iter().collect();
            !(text.contains('-') && text.split('-').all(is_phrase_piece))
        };
        run.clear();
        shaped
    };
    for c in text.chars() {
        if base64(c) {
            run.push(c);
        } else if check(&mut run) {
            return true;
        }
    }
    check(&mut run)
}

/// A written form, spoken form or misrecognition: NFC and trimmed, then every rule of
/// the validation table. `Ok` carries the normalized text.
pub fn term(raw: &str) -> Result<String, ValidationCode> {
    let text = normalize(raw);
    plain_line(&text, MAX_TERM_CHARS)?;
    if text.split_whitespace().count() > MAX_WORDS {
        return Err(ValidationCode::TooManyWords);
    }
    if text.contains(['<', '>']) {
        return Err(ValidationCode::AngleBracket);
    }
    if text.contains("://") || text.to_ascii_lowercase().starts_with("www.") {
        return Err(ValidationCode::Url);
    }
    if text.split_whitespace().any(is_email) {
        return Err(ValidationCode::Email);
    }
    if is_path(&text) {
        return Err(ValidationCode::Path);
    }
    if is_digits_only(&text) {
        return Err(ValidationCode::DigitsOnly);
    }
    if has_long_hex(&text) {
        return Err(ValidationCode::LongHex);
    }
    if is_secret_shaped(&text) {
        return Err(ValidationCode::SecretShaped);
    }
    Ok(text)
}

/// A context cue: a term of at most 32 characters.
pub fn cue(raw: &str) -> Result<String, ValidationCode> {
    let text = term(raw)?;
    if text.chars().count() > MAX_CUE_CHARS {
        return Err(ValidationCode::TooLong);
    }
    Ok(text)
}

/// A list of terms: each validated, duplicates dropped in order, at most `max`.
pub fn terms(raw: &[String], max: usize) -> Result<Vec<String>, ValidationCode> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for item in raw {
        let text = term(item)?;
        if !out.contains(&text) {
            out.push(text);
        }
    }
    if out.len() > max {
        return Err(ValidationCode::TooManyForms);
    }
    Ok(out)
}

/// A note: one line of at most 280 characters. A blank note is no note.
pub fn note(raw: &str) -> Result<Option<String>, ValidationCode> {
    one_line(raw, MAX_NOTE_CHARS)
}

/// An agent's reason: one line of at most 140 characters.
pub fn reason(raw: &str) -> Result<Option<String>, ValidationCode> {
    one_line(raw, MAX_REASON_CHARS)
}

fn one_line(raw: &str, max: usize) -> Result<Option<String>, ValidationCode> {
    let text = normalize(raw);
    if text.is_empty() {
        return Ok(None);
    }
    plain_line(&text, max)?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationCode::*;

    /// The validation table, one row per rule, with the refusal's sentence.
    #[test]
    fn the_validation_table() {
        let sixty_five = "A".repeat(65);
        for (raw, code) in [
            ("", Empty),
            ("   ", Empty),
            ("EP\nAS1", LineBreak),
            ("EP\rAS1", LineBreak),
            ("EPAS1\u{2028}x", LineBreak),
            ("EPAS1\u{2029}x", LineBreak),
            ("EPAS1\u{85}x", LineBreak),
            ("EP\u{7}AS1", ControlCharacter),
            ("EP\u{202E}AS1", ControlCharacter),
            ("EP\u{200B}AS1", ControlCharacter),
            ("<b>EPAS1</b>", AngleBracket),
            ("a > b", AngleBracket),
            (sixty_five.as_str(), TooLong),
            ("one two three four five six seven", TooManyWords),
            ("https://example.org", Url),
            ("www.ucsf.edu", Url),
            ("ftp://host", Url),
            ("someone@ucsf.edu", Email),
            ("write to a.person@example.com", Email),
            ("/etc/passwd", Path),
            ("~/notes", Path),
            ("./build", Path),
            ("../up", Path),
            ("C:\\Users", Path),
            ("a/b/c", Path),
            ("a\\b\\c", Path),
            ("12345", DigitsOnly),
            ("2026-11-04", DigitsOnly),
            ("1.5", DigitsOnly),
            ("(415) 555-0100", DigitsOnly),
            ("deadbeefdeadbeef", LongHex),
            ("sha 0123456789abcdef0", LongHex),
            ("sk-proj-abcdefghij", SecretShaped),
            ("AKIAABCDEFGHIJKL", SecretShaped),
            ("ghp_abcdefghijklm", SecretShaped),
            ("xoxb-1234-abcdefgh", SecretShaped),
            ("QWxhZGRpbjpvcGVuIHNlc2FtZQ9Zabcdefg", SecretShaped),
            // A key with a hyphen: its pieces mix cases and digits, so it is no phrase.
            ("xK9fL2QmZp8-Rt7vN3bW1cY6hJ4sD0aG", SecretShaped),
            ("Hypoxia-Inducible_Factor-1-alpha", SecretShaped),
        ] {
            assert_eq!(term(raw), Err(code), "{raw:?}");
        }
    }

    #[test]
    fn ordinary_terms_pass_normalized() {
        for (raw, want) in [
            ("EPAS1", "EPAS1"),
            ("  UCSF  ", "UCSF"),
            ("HIF-2α", "HIF-2α"),
            ("HIF-1α/ARNT", "HIF-1α/ARNT"),
            ("Cafe\u{301}", "Caf\u{e9}"),
            ("one two three four five six", "one two three four five six"),
            ("COVID-19", "COVID-19"),
            (
                "Chronic-Obstructive-Pulmonary-Disease",
                "Chronic-Obstructive-Pulmonary-Disease",
            ),
            // Hyphenated phrases with a digit, a capital and a small letter, 32 and 33
            // characters long.
            (
                "Hypoxia-Inducible-Factor-1-alpha",
                "Hypoxia-Inducible-Factor-1-alpha",
            ),
            (
                "Interleukin-1-Receptor-Antagonist",
                "Interleukin-1-Receptor-Antagonist",
            ),
            (
                "HIF1A-Mediated-Transcription-Program",
                "HIF1A-Mediated-Transcription-Program",
            ),
            ("IL-6", "IL-6"),
            ("p53", "p53"),
            ("sk-N", "sk-N"),
            ("AKIA", "AKIA"),
        ] {
            assert_eq!(term(raw), Ok(want.to_string()), "{raw:?}");
        }
        assert_eq!(term(&"Z".repeat(64)).map(|t| t.len()), Ok(64));
    }

    #[test]
    fn nfc_is_applied_before_the_length_is_counted() {
        // 64 letters each written as a base letter and a combining accent: 128 code points
        // before NFC, 64 after, so the term passes.
        let decomposed = "e\u{301}".repeat(64);
        assert_eq!(term(&decomposed).map(|t| t.chars().count()), Ok(64));
    }

    #[test]
    fn lists_cues_notes_and_reasons() {
        let nine: Vec<String> = (0..9).map(|i| format!("form {i}")).collect();
        assert_eq!(terms(&nine, MAX_SPOKEN_FORMS), Err(TooManyForms));
        let doubled = vec!["epas one".to_string(), "epas one".to_string()];
        assert_eq!(
            terms(&doubled, MAX_SPOKEN_FORMS),
            Ok(vec!["epas one".into()])
        );
        assert_eq!(cue(&"q".repeat(33)), Err(TooLong));
        assert_eq!(cue("hypoxia"), Ok("hypoxia".into()));
        assert_eq!(note("  "), Ok(None));
        assert_eq!(note("two\nlines"), Err(LineBreak));
        assert_eq!(note(&"n".repeat(281)), Err(TooLong));
        assert_eq!(reason(&"r".repeat(141)), Err(TooLong));
        assert_eq!(
            reason("named in the chat"),
            Ok(Some("named in the chat".into()))
        );
    }

    #[test]
    fn every_code_has_its_catalog_sentence() {
        assert_eq!(TooLong.message(), "Use a term of at most 64 characters.");
        assert_eq!(TooManyWords.message(), "Use a term of at most six words.");
        assert_eq!(LineBreak.message(), "Use one line of plain text.");
        assert_eq!(
            Path.message(),
            "A dictionary term cannot be a web address, an email address or a file path."
        );
        assert_eq!(
            SecretShaped.message(),
            "This looks like an identifier or a key, so it was not added."
        );
        assert_eq!(
            TooManyForms.message(),
            "Use at most eight spoken forms and sixteen misrecognitions for a term."
        );
        for code in ValidationCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
            assert_eq!(ValidationCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(ValidationCode::parse("cap"), None);
    }
}
