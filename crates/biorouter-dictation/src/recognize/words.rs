//! The English word list (`dictionary.md`, "The English word list").
//!
//! `assets/english-words.zst` is a list derived from the English Speller Database at size
//! 60 (US, GB, CA and AU spellings, inflections and contractions), lowercased, with every
//! word the retired name guard matches removed and the hand list `phd uc ucsd crispr rna
//! dna pdf` added. `scripts/dictation/make-english-words.py` builds it from a source pinned
//! by its SHA-256. One word per line in code point order; a word the source lists only
//! capitalized (a proper noun or a brand, `Boston`, `Ventolin`) carries a leading `+`;
//! lines that start with `#` carry the source's copyright and permission notices, which
//! its license asks to travel with every copy.
//!
//! The list drives the `collides` flag, the decoder bonus's start piece filter, the single
//! token guard of the exact key matcher, the fuzzy matcher's English guard and the wordy
//! alias rule. A system word list is never used: it lacks `coding`, `here's`, `aren't`,
//! `encodes` and `cells`, and Windows has none [M].
//!
//! It is decoded once per process, on first use, into one buffer and a sorted index, so a
//! lookup is a binary search with no allocation. A caller on the path from stop to text
//! never pays the decode: the bias set is built when a stream or a file starts, and the
//! worker, whose first bonus table reads the list too, decodes its own copy while its model
//! loads.

use std::sync::OnceLock;

/// The compressed list, embedded in every binary that links this crate.
static PACKED: &[u8] = include_bytes!("../../assets/english-words.zst");

/// The decoded list: every word folded to lower case, in order, and whether some listing of
/// it is common (lower case or capitals) rather than only capitalized.
pub struct WordList {
    text: String,
    /// `(start, end, common)` of each word in `text`, sorted by the word.
    words: Vec<(u32, u32, bool)>,
}

/// How a word is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listing {
    /// Listed in lower case or in capitals: an ordinary word or an abbreviation
    /// (`spoke`, `CD`, `PhD`).
    Common,
    /// Listed only capitalized: a proper noun or a brand (`Boston`, `Ventolin`).
    Proper,
}

impl WordList {
    fn decode(packed: &[u8]) -> WordList {
        // A list that cannot be decoded is an empty one: every guard then treats every
        // token as a non word, which only ever makes correction less willing, never more.
        let text = zstd::stream::decode_all(packed)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default();
        let mut words = Vec::with_capacity(120_000);
        let mut start = 0usize;
        for line in text.split_inclusive('\n') {
            let end = start + line.len();
            let word = line.trim_end_matches('\n');
            if !word.is_empty() && !word.starts_with('#') {
                let (word_start, common) = match word.strip_prefix('+') {
                    Some(_) => (start + 1, false),
                    None => (start, true),
                };
                let word_end = start + word.len();
                if let (Ok(s), Ok(e)) = (u32::try_from(word_start), u32::try_from(word_end)) {
                    words.push((s, e, common));
                }
            }
            start = end;
        }
        WordList { text, words }
    }

    fn word(&self, index: usize) -> &str {
        self.words
            .get(index)
            .and_then(|(s, e, _)| self.text.get(*s as usize..*e as usize))
            .unwrap_or_default()
    }

    /// How `folded` (already lower case) is listed, if it is.
    fn find(&self, folded: &str) -> Option<Listing> {
        let index = self
            .words
            .binary_search_by(|(s, e, _)| {
                self.text
                    .get(*s as usize..*e as usize)
                    .unwrap_or_default()
                    .cmp(folded)
            })
            .ok()?;
        let (_, _, common) = self.words.get(index)?;
        Some(if *common {
            Listing::Common
        } else {
            Listing::Proper
        })
    }

    /// How many words the list holds.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Every word, folded, in order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        (0..self.words.len()).map(|i| self.word(i))
    }

    /// The `#` lines: the source's notices.
    pub fn notice(&self) -> impl Iterator<Item = &str> {
        self.text.lines().filter(|line| line.starts_with('#'))
    }
}

/// The process's word list, decoded on first use.
pub fn list() -> &'static WordList {
    static LIST: OnceLock<WordList> = OnceLock::new();
    LIST.get_or_init(|| WordList::decode(PACKED))
}

/// Decodes the list now, so the first dictation does not. Cheap after the first call.
pub fn warm() {
    let _ = list();
}

/// Lower case, with a typographic apostrophe read as `'`. Allocates only when `word`
/// holds a capital or a typographic apostrophe.
fn folded(word: &str) -> std::borrow::Cow<'_, str> {
    if word
        .chars()
        .any(|c| c.is_uppercase() || c == '\u{2019}' || c == '\u{2018}')
    {
        std::borrow::Cow::Owned(
            word.chars()
                .map(|c| {
                    if matches!(c, '\u{2019}' | '\u{2018}') {
                        '\''
                    } else {
                        c
                    }
                })
                .flat_map(char::to_lowercase)
                .collect(),
        )
    } else {
        std::borrow::Cow::Borrowed(word)
    }
}

/// How `word` is listed, ignoring case, or `None` when it is not a word.
pub fn listing(word: &str) -> Option<Listing> {
    if word.is_empty() {
        return None;
    }
    list().find(&folded(word))
}

/// Whether `word`, ignoring case, is an English word, a proper noun or a listed
/// abbreviation: what the guards that keep correction off ordinary speech ask.
pub fn is_english(word: &str) -> bool {
    listing(word).is_some()
}

/// Whether `word`, ignoring case, is an ordinary word or abbreviation (not only a proper
/// noun): what `collides` asks.
pub fn is_common(word: &str) -> bool {
    listing(word) == Some(Listing::Common)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_decodes_sorted_and_with_its_notice() {
        let list = list();
        assert!(list.len() > 100_000, "{} words", list.len());
        let words: Vec<&str> = list.iter().collect();
        assert!(words.windows(2).all(|w| w[0] < w[1]), "sorted, no repeats");
        assert!(words.iter().all(|w| *w == w.to_lowercase()), "folded");
        let notice: Vec<&str> = list.notice().collect();
        assert!(notice.iter().any(|l| l.contains("Copyright")));
        assert!(notice.iter().any(|l| l.contains("Permission to use")));
    }

    /// The words a system list lacks are here, and so is the hand list.
    #[test]
    fn the_words_correction_needs_are_listed() {
        for word in [
            "coding", "here's", "aren't", "encodes", "cells", "spoke", "clinic", "continue",
            "smoke", "crisper", "Boston", "exon", "ain't", "norms",
        ] {
            assert!(is_english(word), "{word}");
        }
        for word in ["phd", "uc", "ucsd", "crispr", "rna", "dna", "pdf"] {
            assert!(is_common(word), "{word}");
        }
        assert!(is_english("Here\u{2019}s"), "a typographic apostrophe");
        assert!(is_english("SPOKE") && is_common("Spoke"), "case is ignored");
        for word in ["EPAS1", "Biorouter", "omop", "hras", "ntrk", "", "egln"] {
            assert!(!is_english(word), "{word}");
        }
        // A brand the source lists capitalized is English for the guards but does not
        // collide.
        assert_eq!(listing("Ventolin"), Some(Listing::Proper));
        assert_eq!(listing("boston"), Some(Listing::Proper));
        assert_eq!(listing("spoke"), Some(Listing::Common));
    }

    /// R-31: the list passes the retired name guard. The guard's own pattern is read from
    /// the guard, so this test never spells it.
    #[test]
    fn the_word_list_passes_the_retired_name_guard() {
        let guard = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/check-no-goose-block.sh");
        let source = std::fs::read_to_string(&guard).expect("the guard script");
        let line = source
            .lines()
            .find(|l| l.starts_with("ci='"))
            .expect("the guard's ci= line");
        let pattern = line.trim_start_matches("ci='").trim_end_matches('\'');
        // Single words can only trip the guard's plain alternatives; the URL and logo
        // alternatives all hold a character no word in the list has.
        let plain: Vec<&str> = pattern
            .split('|')
            .filter(|alt| alt.chars().all(|c| c.is_ascii_lowercase()))
            .collect();
        assert!(plain.len() >= 4, "the guard's plain words: {plain:?}");
        for word in list().iter() {
            let word = word.replace("mongoose", "");
            for alt in &plain {
                assert!(!word.contains(alt), "the list holds a guarded word");
            }
        }
        for line in list().notice() {
            let line = line.to_lowercase().replace("mongoose", "");
            for alt in &plain {
                assert!(!line.contains(alt), "the notice holds a guarded word");
            }
        }
    }
}
