//! Canonical byte level BPE over the loaded model's own vocabulary (`dictionary.md`, "Token
//! prompt", DD-45).
//!
//! whisper.cpp tokenizes `initial_prompt` text by greedy longest match, so the model copies
//! a segmentation it was never trained on (` EPA|S|1`). On synthetic speech the same list
//! as canonical ids raised turbo from 80 to 93 % pooled strict recall and small.en from 85
//! to 90 % [M]. So the worker encodes the prompt itself and passes the ids as
//! `prompt_tokens`, with `initial_prompt` unset.
//!
//! The ranks are the model's token ids: every multi byte token of small.en and turbo
//! splits into two lower id tokens, and turbo's ids equal `multilingual.tiktoken`'s ranks
//! except at id 188 [M]. `.en` models use the GPT-2 vocabulary. The pre split is GPT-2's
//! pattern, the one both Whisper encodings use:
//!
//! ```text
//! 's|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+
//! ```
//!
//! written out by hand rather than through a regular expression crate, because the
//! standard `regex` crate has no look ahead and this is the only pattern. `\p{L}` is read
//! as a letter that is not numeric and `\p{N}` as a numeric character, which differ from
//! the Unicode classes only for a few combining marks some scripts count as alphabetic.
//! No tokenizer file ships: the vocabulary is read from the loaded model.

use std::collections::HashMap;

/// A model's text vocabulary: the bytes of every id below end of text.
pub struct Vocab {
    bytes: Vec<Box<[u8]>>,
    ids: HashMap<Box<[u8]>, i32>,
    eot: i32,
    longest: usize,
}

impl Vocab {
    /// The vocabulary from the bytes of ids `0..eot`, in order. An empty token (whisper.cpp
    /// keeps a few) maps to no bytes and is never produced.
    pub fn new(tokens: Vec<Vec<u8>>, eot: i32) -> Vocab {
        let mut ids = HashMap::with_capacity(tokens.len());
        let mut bytes = Vec::with_capacity(tokens.len());
        let mut longest = 0;
        for (id, token) in tokens.into_iter().enumerate() {
            let token: Box<[u8]> = token.into_boxed_slice();
            longest = longest.max(token.len());
            if !token.is_empty() {
                // The lowest id wins a repeated byte string, as a rank should.
                ids.entry(token.clone()).or_insert(id as i32);
            }
            bytes.push(token);
        }
        Vocab {
            bytes,
            ids,
            eot,
            longest,
        }
    }

    /// The end of text id: every id at or above it is a special or timestamp token.
    pub fn eot(&self) -> i32 {
        self.eot
    }

    /// The bytes of `id`, for an id below end of text.
    pub fn token(&self, id: i32) -> Option<&[u8]> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.bytes.get(i))
            .map(|b| &**b)
    }

    /// The id whose bytes are exactly `bytes`.
    pub fn id(&self, bytes: &[u8]) -> Option<i32> {
        self.ids.get(bytes).copied()
    }

    /// The longest token, in bytes.
    pub fn longest(&self) -> usize {
        self.longest
    }

    /// How many ids the vocabulary holds below end of text.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// `text` as canonical token ids, or `None` when some byte has no token.
    pub fn encode(&self, text: &str) -> Option<Vec<i32>> {
        let mut out = Vec::with_capacity(text.len() / 2 + 1);
        for piece in pre_split(text) {
            self.merge(piece.as_bytes(), &mut out)?;
        }
        Some(out)
    }

    /// Byte pair merges of one piece: start from single bytes and merge the adjacent pair
    /// with the lowest rank until no pair is a token.
    fn merge(&self, piece: &[u8], out: &mut Vec<i32>) -> Option<()> {
        if let Some(id) = self.id(piece) {
            out.push(id);
            return Some(());
        }
        let mut parts: Vec<(usize, usize)> = (0..piece.len()).map(|i| (i, i + 1)).collect();
        loop {
            let mut best: Option<(i32, usize)> = None;
            for (k, pair) in parts.windows(2).enumerate() {
                let (start, end) = (pair[0].0, pair[1].1);
                if let Some(rank) = piece.get(start..end).and_then(|b| self.id(b)) {
                    if best.is_none_or(|(r, _)| rank < r) {
                        best = Some((rank, k));
                    }
                }
            }
            let Some((_, k)) = best else {
                break;
            };
            let end = parts[k + 1].1;
            parts[k].1 = end;
            parts.remove(k + 1);
        }
        for (start, end) in parts {
            out.push(self.id(piece.get(start..end)?)?);
        }
        Some(())
    }
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic() && !c.is_numeric()
}

fn is_number(c: char) -> bool {
    c.is_numeric()
}

fn is_other(c: char) -> bool {
    !c.is_whitespace() && !is_letter(c) && !is_number(c)
}

const CONTRACTIONS: [&str; 7] = ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d"];

/// GPT-2's pre split, as Whisper's encodings apply it.
pub fn pre_split(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let byte_at = |i: usize| chars.get(i).map_or(text.len(), |(b, _)| *b);
    let char_at = |i: usize| chars.get(i).map(|(_, c)| *c);
    let run = |from: usize, class: fn(char) -> bool| -> usize {
        let mut j = from;
        while char_at(j).is_some_and(class) {
            j += 1;
        }
        j
    };
    let mut pieces = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let rest = text.get(byte_at(i)..).unwrap_or_default();
        if let Some(c) = CONTRACTIONS.iter().find(|c| rest.starts_with(**c)) {
            let end = i + c.chars().count();
            pieces.push(text.get(byte_at(i)..byte_at(end)).unwrap_or_default());
            i = end;
            continue;
        }
        let c = char_at(i).unwrap_or(' ');
        // ` ?\p{L}+`, ` ?\p{N}+`, ` ?[^\s\p{L}\p{N}]+`, the space optional.
        let (start_class, from) = if c == ' ' {
            match char_at(i + 1) {
                Some(n) if is_letter(n) => (Some(is_letter as fn(char) -> bool), i + 1),
                Some(n) if is_number(n) => (Some(is_number as fn(char) -> bool), i + 1),
                Some(n) if is_other(n) => (Some(is_other as fn(char) -> bool), i + 1),
                _ => (None, i),
            }
        } else if is_letter(c) {
            (Some(is_letter as fn(char) -> bool), i)
        } else if is_number(c) {
            (Some(is_number as fn(char) -> bool), i)
        } else if is_other(c) {
            (Some(is_other as fn(char) -> bool), i)
        } else {
            (None, i)
        };
        if let Some(class) = start_class {
            let end = run(from, class);
            pieces.push(text.get(byte_at(i)..byte_at(end)).unwrap_or_default());
            i = end;
            continue;
        }
        // `\s+(?!\S)`, then `\s+`: a whitespace run, leaving its last character to the
        // next piece when one follows.
        let end = run(i, char::is_whitespace);
        let end = if end < chars.len() && end - i >= 2 {
            end - 1
        } else {
            end
        };
        pieces.push(text.get(byte_at(i)..byte_at(end)).unwrap_or_default());
        i = end.max(i + 1);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pre_split_is_gpt2s() {
        assert_eq!(
            pre_split("EPAS1, Biorouter"),
            ["EPAS", "1", ",", " Biorouter"]
        );
        assert_eq!(pre_split("HIF-2α"), ["HIF", "-", "2", "α"]);
        assert_eq!(pre_split("here's it"), ["here", "'s", " it"]);
        assert_eq!(pre_split("a  b"), ["a", " ", " b"]);
        assert_eq!(pre_split("end  "), ["end", "  "]);
        assert_eq!(pre_split("C++, x"), ["C", "++,", " x"]);
        assert_eq!(pre_split(" 42"), [" 42"]);
        assert_eq!(pre_split("a\nb"), ["a", "\n", "b"]);
        assert_eq!(pre_split(""), Vec::<&str>::new());
    }

    /// A toy vocabulary: every byte, then merges whose ids are their ranks.
    fn toy() -> Vocab {
        let mut tokens: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
        for merge in ["ab", "bc", "abc", " a", " abc"] {
            tokens.push(merge.as_bytes().to_vec());
        }
        let eot = tokens.len() as i32;
        Vocab::new(tokens, eot)
    }

    #[test]
    fn merges_follow_the_ranks() {
        let v = toy();
        // "abc": "ab" (256) merges before "bc" (257), then "abc" (258).
        assert_eq!(v.encode("abc"), Some(vec![258]));
        // " abcd" is one piece: "ab" (256) merges first, then "abc" (258), then " abc"
        // (260); "d" stays alone.
        assert_eq!(v.encode(" abcd"), Some(vec![260, i32::from(b'd')]));
        assert_eq!(v.id(b"ab"), Some(256));
        assert_eq!(v.token(258), Some(&b"abc"[..]));
        assert_eq!(v.longest(), 4);
        assert_eq!(v.eot(), 261);
        let empty = Vocab::new(vec![b"a".to_vec()], 1);
        assert_eq!(empty.encode("b"), None, "a byte with no token");
    }
}

/// Parity on the real vocabularies: the `for-tests` models carry the full vocabularies of
/// `base.en` (which `small.en` shares) and `tiny` (which turbo shares), with no tensors.
#[cfg(test)]
mod real_vocabularies {
    use crate::build_info::fixtures;
    use crate::engine::{test_support::engine_lock, Engine, LoadOptions};
    use crate::recognize::prompt;

    /// The synthetic benchmark's 14 terms, as its prompt listed them.
    const LIST_14: &str = "EPAS1, EGLN1, HIF-2α, UCSF, Baranzini, SPOKE, OMOP, natalizumab, \
                           ocrelizumab, scRNA-seq, CRISPRi, Biorouter, PHD2, VHL";
    /// The PriMock57 subset's 26 drug terms (`eval-primock57-subset-r1`'s `terms.json`),
    /// most frequent last.
    const LIST_26: [&str; 26] = [
        "Lemsip",
        "Cetraben",
        "Modulite",
        "lisinopril",
        "EpiPen",
        "methotrexate",
        "metformin",
        "trimethoprim",
        "nitrofurantoin",
        "fexofenadine",
        "Piriton",
        "loratadine",
        "sertraline",
        "codeine",
        "hydrocortisone",
        "thyroxine",
        "furosemide",
        "naproxen",
        "salbutamol",
        "clindamycin",
        "Implanon",
        "Clenil",
        "Ventolin",
        "aspirin",
        "ibuprofen",
        "paracetamol",
    ];
    /// The research set's 37 gene terms (34 symbols with quizartinib, LUXTURNA and
    /// schwannomatosis), most frequent last.
    const LIST_37: [&str; 37] = [
        "schwannomatosis",
        "LUXTURNA",
        "quizartinib",
        "BRCA1",
        "APOB",
        "ASXL1",
        "IDH2",
        "IDH1",
        "DNMT3A",
        "FBP1",
        "ARNT",
        "GNA11",
        "HRAS",
        "RPE65",
        "SLC16A11",
        "MTOR",
        "PRF1",
        "GZMA",
        "GNAQ",
        "IGF1R",
        "PROKR2",
        "A2ML1",
        "PNPLA3",
        "CXCR4",
        "KCNQ3",
        "HK2",
        "PRAME",
        "LRRC15",
        "ALOX5AP",
        "NF1",
        "CD4",
        "FLT3",
        "NRAS",
        "KCNQ2",
        "BRAF",
        "CCR5",
        "NF2",
    ];

    fn load(relative: &str) -> Engine {
        let model = fixtures::path(relative);
        let vad = fixtures::path(fixtures::SILERO);
        Engine::load(&LoadOptions {
            model_path: &model,
            model_size: std::fs::metadata(&model).expect("fixture").len(),
            vad_path: &vad,
            use_gpu: false,
            n_threads: 2,
        })
        .expect("the fixture loads")
    }

    /// `whisper_tokenize` gives the counts measured on the real models (111, 149 and 62 on
    /// the English vocabulary, 119, 160 and 63 on the multilingual one), so the fixtures'
    /// vocabularies are the models'. The canonical counts are this encoder's, which equal
    /// the research's reference encoder's [M]; the golden splits are committed as numbers
    /// per vocabulary, because the two vocabularies share almost no ids.
    #[test]
    fn bpe_parity_on_the_fixture_vocabularies() {
        let _lock = engine_lock();
        let lists = [LIST_14.to_string(), LIST_26.join(", "), LIST_37.join(", ")];
        for (model, whisper_counts, canonical_counts, epas, biorouter) in [
            (
                fixtures::BASE_EN,
                [62, 111, 149],
                [62, 118, 152],
                [14724, 1921, 16],
                [347, 1504, 39605],
            ),
            (
                fixtures::TINY,
                [63, 119, 160],
                [63, 123, 160],
                [25330, 3160, 16],
                [363, 1973, 23985],
            ),
        ] {
            let engine = load(model);
            let vocab = engine.vocabulary();
            assert_eq!(vocab.eot(), engine.token_eot());
            for (i, list) in lists.iter().enumerate() {
                let whisper = engine.whisper_tokenize(list).expect("whisper_tokenize");
                let canonical = vocab.encode(list).expect("canonical");
                assert_eq!(whisper.len(), whisper_counts[i], "{model} list {i}");
                assert_eq!(canonical.len(), canonical_counts[i], "{model} list {i}");
            }
            let pieces = |ids: &[i32]| -> Vec<String> {
                ids.iter()
                    .map(|id| {
                        String::from_utf8_lossy(vocab.token(*id).unwrap_or_default()).into_owned()
                    })
                    .collect()
            };
            let ids = vocab.encode(" EPAS1").expect("encodes");
            assert_eq!(ids, epas, "{model}");
            assert_eq!(pieces(&ids), [" EP", "AS", "1"]);
            let ids = vocab.encode(" Biorouter").expect("encodes");
            assert_eq!(ids, biorouter, "{model}");
            assert_eq!(pieces(&ids), [" B", "ior", "outer"]);
            // whisper.cpp's greedy match splits it otherwise, which is why ids are sent.
            assert_ne!(engine.whisper_tokenize(" EPAS1").expect("tokenize"), epas);
        }
    }

    /// A 600 token dictionary trims to at most 150 tokens with the top entry last.
    #[test]
    fn a_long_prompt_trims_from_the_head() {
        let _lock = engine_lock();
        let engine = load(fixtures::BASE_EN);
        let vocab = engine.vocabulary();
        let mut terms: Vec<String> = Vec::new();
        for round in 0..6 {
            terms.extend(
                LIST_26
                    .iter()
                    .map(|t| format!("{t}{}", ["", "a", "b", "c", "d", "e"][round])),
            );
        }
        terms.push("EPAS1".into());
        let whole = vocab.encode(&terms.join(", ")).expect("encodes").len();
        assert!(whole >= 600, "{whole} tokens");
        let (ids, used) = prompt::fit(&terms, 150, |text| vocab.encode(text));
        assert!(ids.len() <= 150, "{} tokens", ids.len());
        assert!(used >= 20 && used < terms.len(), "{used} terms");
        let tail = terms[terms.len() - used..].join(", ");
        assert_eq!(ids, vocab.encode(&tail).expect("encodes"));
        assert!(tail.ends_with("EPAS1"), "the top entry is last");
    }
}
