//! The engine's words as text: joining what the decoder wrote, and the whole output
//! filter (DD-48).
//!
//! Whisper learned from captioned video, and over silence and noise it writes what
//! captions hold where nobody speaks: a run of bracketed tags (`[BLANK_AUDIO]`,
//! `(music)`), "you", "Thank you." and caption credits in several languages. The voice
//! detector runs before every decode and removes most of it [M]; this filter removes what
//! is left, and only when it is all of the output. A real sentence that contains "you",
//! or a parenthesis, is never touched.

/// The phrases removed when one of them is the whole output, compared after the
/// characters that are not letters or digits are trimmed from both ends, with case
/// folded and inner whitespace collapsed (DD-48): "you", "Thank you.", "Thank you for
/// watching!", "Okay.", "so", "and", ".", "Grazie", "Grazie a tutti", "Sous-titrage
/// Société Radio-Canada", "Gracias por ver el video" and "Altyazı M.K.". "." trims to
/// nothing, which the filter treats as no speech in any case.
pub const NONSPEECH_PHRASES: &[&str] = &[
    "you",
    "thank you",
    "thank you for watching",
    "okay",
    "so",
    "and",
    "grazie",
    "grazie a tutti",
    "sous-titrage société radio-canada",
    "gracias por ver el video",
    "altyazı m.k",
];

/// `text` with every character that is not a letter or a digit trimmed from both ends.
pub fn trim_punctuation(text: &str) -> &str {
    text.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Lower case, with runs of whitespace collapsed to one space.
fn fold(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `text` is one or more bracketed or parenthesized tags and nothing else, such
/// as `[BLANK_AUDIO]` or `(music) [Applause]`. Characters between tags that are not
/// letters or digits (spaces, dashes, music notes) are allowed; a letter or digit
/// outside a tag is speech. A tag holds at least one letter or digit and no nested
/// bracket.
pub fn is_tag_run(text: &str) -> bool {
    let mut rest = text.trim();
    let mut tags = 0usize;
    while let Some(first) = rest.chars().next() {
        let close = match first {
            '[' => ']',
            '(' => ')',
            c if c.is_alphanumeric() => return false,
            c => {
                rest = rest.get(c.len_utf8()..).unwrap_or_default();
                continue;
            }
        };
        let body = rest.get(1..).unwrap_or_default();
        let Some(end) = body.find(close) else {
            return false;
        };
        let inner = body.get(..end).unwrap_or_default();
        if inner.contains(['[', '(', '\n']) || trim_punctuation(inner).is_empty() {
            return false;
        }
        tags += 1;
        rest = body.get(end + close.len_utf8()..).unwrap_or_default();
    }
    tags > 0
}

/// The whole output filter: true when `text` holds nothing a person said. That is an
/// output with no letter or digit, a run of tags, or exactly one of
/// [`NONSPEECH_PHRASES`].
pub fn is_nonspeech(text: &str) -> bool {
    let trimmed = trim_punctuation(text);
    if trimmed.is_empty() || is_tag_run(text) {
        return true;
    }
    NONSPEECH_PHRASES.contains(&fold(trimmed).as_str())
}

/// The output after the whole output filter: empty when it holds no speech, else the
/// text with its whitespace collapsed.
pub fn filter_whole_output(text: &str) -> String {
    if is_nonspeech(text) {
        return String::new();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether one piece the decoder wrote (a Whisper segment) carries words: not empty, not
/// punctuation alone and not a run of tags. Pieces that fail are dropped before the
/// pieces are joined, so a tag between two sentences never reaches the text. The phrase
/// list applies only to the whole output, never to a piece.
pub fn piece_has_words(piece: &str) -> bool {
    !trim_punctuation(piece).is_empty() && !is_tag_run(piece)
}

/// The pieces joined in order with one space, each trimmed, those without words left out.
///
/// Each piece is a decode of its own, which starts with no context (DD-47's
/// `no_context`), so a piece that follows a finished sentence sometimes begins in lower
/// case ("phenotypes. normalize the counts"). The first word of such a piece is
/// capitalized ([`capitalize_after_sentence`]); inside a piece, and after a piece that
/// does not end a sentence, nothing changes.
pub fn join<'a>(pieces: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for piece in pieces.into_iter().filter(|piece| piece_has_words(piece)) {
        let mut words = piece.split_whitespace();
        let Some(first) = words.next() else {
            continue;
        };
        if out.is_empty() {
            out.push_str(first);
        } else {
            let ended = out.rsplit(' ').next().is_some_and(ends_sentence);
            out.push(' ');
            if ended {
                out.push_str(&capitalize_after_sentence(first));
            } else {
                out.push_str(first);
            }
        }
        for word in words {
            out.push(' ');
            out.push_str(word);
        }
    }
    out
}

/// Short words written with a period that do not end a sentence ("Dr. Patel", "vs. the").
const ABBREVIATIONS: &[&str] = &["dr", "mr", "mrs", "ms", "prof", "st", "vs", "cf", "approx"];

/// Whether `word`, the last word written, ends a sentence: a full stop, a question mark or
/// an exclamation mark, before any closing quotes or brackets. An ellipsis trails off
/// rather than ends, and the period of an abbreviation, of a single letter ("J.") or of a
/// word with a period inside it ("U.S.", "e.g.", "a.m.") is not a sentence end; a decimal
/// point ("2.5.") is not such a period.
fn ends_sentence(word: &str) -> bool {
    let bare = word.trim_end_matches(['"', '\'', ')', ']', '\u{201d}', '\u{2019}']);
    if bare.ends_with('?') || bare.ends_with('!') {
        return true;
    }
    let Some(stem) = bare.strip_suffix('.') else {
        return false;
    };
    let stem = stem.trim_start_matches(['"', '\'', '(', '[', '\u{201c}', '\u{2018}']);
    let chars: Vec<char> = stem.chars().collect();
    let digit_at = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(|c| c.is_ascii_digit())
    };
    let inner_period = chars
        .iter()
        .enumerate()
        .any(|(i, &c)| c == '.' && !(digit_at(i.checked_sub(1)) && digit_at(Some(i + 1))));
    if stem.is_empty() || stem.ends_with('\u{2026}') || inner_period {
        return false;
    }
    let lower = stem.to_lowercase();
    if chars.len() == 1 && chars.iter().all(|c| c.is_alphabetic()) {
        return false;
    }
    !ABBREVIATIONS.contains(&lower.as_str())
}

/// `word` with its first letter capitalized, when the word is written all in lower case
/// ("normalize", "metformin"). A word with a capital or a digit after its first letter
/// ("mRNA", "pH", "eGFR", "scRNA-seq", "x2") is a spelling to keep, so it is left as it
/// is, as is a word that does not start with a letter.
fn capitalize_after_sentence(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = chars.as_str();
    let plain = first.is_lowercase()
        && rest
            .chars()
            .all(|c| !c.is_alphanumeric() || (c.is_alphabetic() && !c.is_uppercase()));
    if !plain {
        return word.to_string();
    }
    first.to_uppercase().chain(rest.chars()).collect()
}

/// The words of `text` for the rate checks: the whitespace separated tokens that hold a
/// letter or a digit, so "-" and "..." do not count.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DD-48's list, as Whisper writes it: each becomes empty, with the punctuation and
    /// case it is written in, and with neighbouring whitespace.
    #[test]
    fn every_listed_phrase_and_tag_run_becomes_empty() {
        for output in [
            "you",
            " You.",
            "Thank you.",
            "thank you",
            "Thank you for watching!",
            "Okay.",
            "okay",
            "so",
            "So.",
            "and",
            ".",
            "...",
            "",
            "   ",
            "Grazie",
            "Grazie a tutti",
            "Grazie a tutti.",
            "Sous-titrage Société Radio-Canada",
            "Gracias por ver el video",
            "Gracias por ver el video.",
            "Altyazı M.K.",
            "[BLANK_AUDIO]",
            " [Music]",
            "(music)",
            "(upbeat music) [Applause]",
            "[MUSIC PLAYING] - [Laughter]",
            "♪ ♪",
            "♪ (piano music) ♪",
            "\u{2014}",
        ] {
            assert!(is_nonspeech(output), "{output:?} must be filtered");
            assert_eq!(filter_whole_output(output), "", "{output:?}");
        }
    }

    /// Real speech is kept, including speech made of a listed word and more, speech in
    /// parentheses and speech beside a tag.
    #[test]
    fn real_sentences_stay() {
        for output in [
            "Thank you, see you tomorrow.",
            "Did you take the ventolin?",
            "you know",
            "So we start metformin.",
            "and then",
            "Okay, increase the dose.",
            "Start sertraline (50 mg) tomorrow.",
            "(laughs) Yes.",
            "[Music] Hello there.",
            "EPAS1",
            "HLA-B*57:01",
            "2",
            "Thank you very much.",
            "Grazie mille, dottore.",
        ] {
            assert!(!is_nonspeech(output), "{output:?} must stay");
            assert_eq!(
                filter_whole_output(output),
                output.split_whitespace().collect::<Vec<_>>().join(" ")
            );
        }
    }

    #[test]
    fn tag_runs_need_closed_tags_with_words() {
        assert!(is_tag_run("[BLANK_AUDIO]"));
        assert!(is_tag_run("[Music] (applause)"));
        assert!(!is_tag_run("[Music"));
        assert!(!is_tag_run("[]"));
        assert!(!is_tag_run("( )"));
        assert!(!is_tag_run("[a [b]]"));
        assert!(!is_tag_run("[Music] yes"));
        assert!(!is_tag_run("ok [Music]"));
        assert!(!is_tag_run(""));
        assert!(!is_tag_run("\u{2014}"));
    }

    #[test]
    fn pieces_are_joined_without_tags_and_with_one_space() {
        let pieces = [
            " And so my fellow Americans,",
            " [BLANK_AUDIO]",
            "",
            "  ask not what your country can do for you,",
            " ...",
            "ask what you can do for your country.",
        ];
        assert_eq!(
            join(pieces),
            "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country."
        );
        // A listed phrase as one piece of a longer output is speech.
        assert_eq!(
            join(["Okay.", " Start metformin."]),
            "Okay. Start metformin."
        );
        assert_eq!(join(["[Music]", "(applause)"]), "");
    }

    /// A piece that follows a finished sentence starts one, so a lower case first word is
    /// capitalized there (`small.en` wrote "phenotypes. normalize the ..."). Never inside a
    /// piece, never after a piece that does not end a sentence, and never a spelling that
    /// is not all lower case.
    #[test]
    fn a_piece_after_a_sentence_end_starts_with_a_capital() {
        let cases: &[(&[&str], &str)] = &[
            (
                &["We saw three phenotypes.", "normalize the counts first."],
                "We saw three phenotypes. Normalize the counts first.",
            ),
            (&["Is it EPAS1?", "yes, it is."], "Is it EPAS1? Yes, it is."),
            (
                &["Stop the infusion!", "then call."],
                "Stop the infusion! Then call.",
            ),
            (
                &["He said \"done.\"", "next patient."],
                "He said \"done.\" Next patient.",
            ),
            (
                &["We rule it out (for now.)", "then image."],
                "We rule it out (for now.) Then image.",
            ),
            (
                &["Increase to 2.5.", "review in a week."],
                "Increase to 2.5. Review in a week.",
            ),
            // Inside a piece, nothing changes.
            (&["one. two three."], "one. two three."),
            // Not a sentence end: no period, a comma, an ellipsis, an abbreviation, an
            // initial, a period inside the word.
            (
                &["We start metformin", "and review."],
                "We start metformin and review.",
            ),
            (&["After the scan,", "we call."], "After the scan, we call."),
            (&["And then...", "we waited."], "And then... we waited."),
            (
                &["And then\u{2026}", "we waited."],
                "And then\u{2026} we waited.",
            ),
            (&["Ask Dr.", "patel to review."], "Ask Dr. patel to review."),
            (&["Compare vs.", "baseline."], "Compare vs. baseline."),
            (
                &["Signed by J.", "smith today."],
                "Signed by J. smith today.",
            ),
            (
                &["Flew in from the U.S.", "last week."],
                "Flew in from the U.S. last week.",
            ),
            (
                &["Use a statin, e.g.", "atorvastatin."],
                "Use a statin, e.g. atorvastatin.",
            ),
            // Spellings that are not all lower case stay as written.
            (
                &["That is settled.", "mRNA levels rose."],
                "That is settled. mRNA levels rose.",
            ),
            (&["Check it.", "pH was 7.2."], "Check it. pH was 7.2."),
            (&["Next.", "eGFR fell."], "Next. eGFR fell."),
            (&["Done.", "scRNA-seq next."], "Done. scRNA-seq next."),
            (&["Done.", "x2 daily."], "Done. x2 daily."),
            (&["Done.", "x-ray today."], "Done. X-ray today."),
            (&["Done.", "50 mg daily."], "Done. 50 mg daily."),
            (&["Gut.", "\u{fc}ber alles."], "Gut. \u{dc}ber alles."),
            // A piece without words between two others changes nothing.
            (
                &["Okay.", "[BLANK_AUDIO]", "start metformin."],
                "Okay. Start metformin.",
            ),
        ];
        for (pieces, joined) in cases {
            assert_eq!(join(pieces.iter().copied()), *joined, "{pieces:?}");
        }
    }

    #[test]
    fn words_hold_a_letter_or_a_digit() {
        assert_eq!(word_count("Start 50 mg - of sertraline ..."), 5);
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("HLA-B*57:01, EPAS1."), 2);
    }
}
