//! Correction after decoding (`dictionary.md`, "Correction after decoding", DD-49).
//!
//! One pass over the decoded text, in four steps, every change recorded as `{start, end,
//! from, to, stage, entry_id}`:
//!
//! 1. **Protect.** Spans inside backticks, URLs, emails, file paths, and any token holding
//!    `@ / \ _ = : + # < > * % | ^ ~ &` or an inner `.` are never changed.
//! 2. **Aliases and misrecognitions**, through an `aho-corasick` automaton over NFC, case
//!    folded text, leftmost longest, whole words only, never across `, . ; : ! ?`. Only
//!    misrecognitions seen twice or more take part. A wordy spoken form (every token an
//!    English word: `oh mop`, `bio router`) applies only when its entry is in the request's
//!    context or one of its cues occurs in the utterance.
//! 3. **Normalized exact keys** ([`super::key`]) over windows of 1 to `min(8, letters + 1)`
//!    tokens that do not cross punctuation, with the guards below.
//! 4. **Guarded fuzzy matching** for the fuzzy set only: the `match = fuzzy` entries among
//!    the top 30, never an entry of 5 characters or fewer, with a letter and digit shape,
//!    or that collides.
//!
//! The text inserted is always the entry's written form in its own case. A span one step
//! changed is never changed again. Everything a request needs is built when the request is
//! created ([`Corrector::new`]), so correcting at stop costs one pass over the text. What
//! does not depend on the request ([`CorrectionIndex`]) can be built once per set and
//! shared ([`Corrector::with_index`]).
//!
//! Two guards go beyond the plan's list, each the smallest one its acceptance needs, and
//! both are the wordy alias rule applied to step 3: a window of two or more tokens whose
//! every token is an English word (a small single letter counts as a word, a capital one
//! is a spelled letter) is rewritten only when its entry is in context or a cue occurs.
//! Without it the exact key of `Biorouter` rewrote `The bio router in the closet`, and
//! the key of `CRISPRi` rewrote `CRISPR i think`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aho_corasick::{AhoCorasick, MatchKind};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use super::key::{form_key, form_tokens, window_key, Key};
use super::select::RequestPlan;
use super::{words, EligibleEntries, RecognitionEntry};
use crate::store::MatchPolicy;

/// Which step made a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Alias,
    Exact,
    Fuzzy,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Alias => "alias",
            Stage::Exact => "exact",
            Stage::Fuzzy => "fuzzy",
        }
    }
}

/// One change. `start` and `end` count characters (Unicode scalar values) of the final
/// text and bound `to`; `from` is what the decoder wrote there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub from: String,
    pub to: String,
    pub stage: Stage,
    pub entry_id: String,
}

/// A corrected text and its changes, in text order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Corrected {
    pub text: String,
    pub changes: Vec<Change>,
}

/// Characters that make a token protected wherever they occur in it.
const PROTECTING: &[char] = &[
    '@', '/', '\\', '_', '=', ':', '+', '#', '<', '>', '*', '%', '|', '^', '~', '&',
];
/// Punctuation no alias, window or span crosses.
const BREAKING: &[char] = &[',', '.', ';', ':', '!', '?'];
/// Punctuation that may open or close a token without belonging to it.
const EDGE: &[char] = &[
    ',', '.', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '"', '\'', '\u{201C}', '\u{201D}',
    '\u{2018}', '\u{2019}',
];
/// The longest window the exact key matcher reads.
const MAX_WINDOW: usize = 8;
/// The longest span the fuzzy matcher reads.
const MAX_FUZZY_SPAN: usize = 3;

/// Fuzzy matching's thresholds (DD-49).
const FUZZY_SIMILARITY: f64 = 0.80;
const FUZZY_SIMILARITY_WITH_SOUND: f64 = 0.60;
const SOUND_SIMILARITY: f64 = 0.75;
const ABSTAIN_WITHIN: f64 = 0.04;
const MIN_FUZZY_KEY: usize = 4;
const MIN_PROMPT_PIECE: usize = 4;

/// Measured near collisions fuzzy matching never makes, in either direction: UCSD and
/// UCSF, NTRK and NRAS, exon and EGLN1, CRISPR and CRISPRi, clinic and Clenil, continue
/// and codeine, smoke and SPOKE, coding and codeine [M].
const STOP_PAIRS: [(&str, &str); 8] = [
    ("ucsd", "ucsf"),
    ("ntrk", "nras"),
    ("exon", "egln1"),
    ("crispr", "crispri"),
    ("clinic", "clenil"),
    ("continue", "codeine"),
    ("smoke", "spoke"),
    ("coding", "codeine"),
];

/// Words a span of several tokens never starts or ends with.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "of", "and", "or", "but", "to", "in", "on", "at", "for", "with", "by",
    "from", "as", "is", "it", "its", "be", "was", "are", "were", "this", "that", "then", "than",
    "so", "if", "not", "no", "do", "we", "you", "he", "she", "they", "i", "my", "our", "your",
    "his", "her", "their", "me", "him", "us", "them",
];

/// The rows a [`CorrectionIndex`] reads: an eligible set's own, shared, or a copy for this
/// crate's tests.
enum Rows {
    Eligible(EligibleEntries),
    #[cfg(test)]
    Copied(Vec<RecognitionEntry>),
}

impl Rows {
    fn get(&self) -> &[RecognitionEntry] {
        match self {
            Rows::Eligible(eligible) => eligible.entries(),
            #[cfg(test)]
            Rows::Copied(entries) => entries,
        }
    }
}

/// What correction reads from a set that no request changes: the alias automaton, the key
/// maps, the written forms and the cues. Built once per set and shared by every request over
/// the same rows (PERF-1): at 5,000 rows with a spoken form and a misrecognition each it
/// takes 16.9 to 19.0 ms to build in a release build, where a request's own part takes
/// about 1.4 µs [M, 2026-11-07]. Usage counters are not read here, so a set rebuilt only
/// because they moved can keep it.
pub struct CorrectionIndex {
    rows: Rows,
    /// The alias automaton and, per pattern, its entry and whether it is wordy.
    aliases: Option<AhoCorasick>,
    alias_targets: Vec<(usize, bool)>,
    /// Exact keys: key to (entry, the matched form's key length). A key two entries share
    /// with different written forms is left out.
    keys: HashMap<String, (usize, usize)>,
    /// Every key any entry has, for the fuzzy matcher's "a correctly spelled other entry".
    all_keys: HashSet<String>,
    /// Every written form, exactly.
    written: HashSet<String>,
    /// Per entry, its cues as lower case word phrases.
    cues: Vec<Vec<String>>,
}

/// What one request's correction reads, built when the request is created: the set's
/// [`CorrectionIndex`] and what the request's ranking adds, which entries are in its
/// context and its fuzzy set.
pub struct Corrector {
    index: Arc<CorrectionIndex>,
    in_context: Vec<bool>,
    /// The fuzzy set: entry, its key, its sound codes.
    fuzzy: Vec<(usize, String, (String, String))>,
}

impl CorrectionIndex {
    /// The index of `eligible`'s rows. It takes an [`EligibleEntries`], never raw rows, so
    /// correction reads exactly the set the prompt and the bonus read (DD-36).
    pub fn new(eligible: &EligibleEntries) -> CorrectionIndex {
        CorrectionIndex::build(Rows::Eligible(eligible.clone()))
    }

    /// [`CorrectionIndex::new`] over a slice, for this crate's tests.
    #[cfg(test)]
    pub(crate) fn over(entries: &[RecognitionEntry]) -> CorrectionIndex {
        CorrectionIndex::build(Rows::Copied(entries.to_vec()))
    }

    /// How many entries it indexes.
    pub fn len(&self) -> usize {
        self.rows.get().len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.get().is_empty()
    }

    fn build(rows: Rows) -> CorrectionIndex {
        let entries = rows.get();
        let correctable = |e: &RecognitionEntry| e.match_policy != MatchPolicy::PromptOnly;

        let mut patterns: Vec<String> = Vec::new();
        let mut targets: Vec<(usize, bool)> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut ambiguous: HashSet<String> = HashSet::new();
        for (i, entry) in entries.iter().enumerate().filter(|(_, e)| correctable(e)) {
            let spoken = entry.spoken_forms.iter().map(|f| (f, true));
            let misheard = entry.misrecognitions.iter().map(|f| (f, false));
            for (form, is_spoken) in spoken.chain(misheard) {
                let pattern: String = form.nfc().flat_map(char::to_lowercase).collect();
                let pattern = pattern.trim().to_string();
                if pattern.is_empty() || pattern.contains(BREAKING) {
                    continue;
                }
                match seen.get(&pattern) {
                    Some(&other) if entries[other].written != entry.written => {
                        ambiguous.insert(pattern);
                    }
                    Some(_) => {}
                    None => {
                        seen.insert(pattern.clone(), i);
                        // A misrecognition the person's corrections made twice applies
                        // whatever its words; a wordy spoken form needs context.
                        targets.push((i, is_spoken && is_wordy(&form_tokens(form))));
                        patterns.push(pattern);
                    }
                }
            }
        }
        let (patterns, targets): (Vec<String>, Vec<(usize, bool)>) = patterns
            .into_iter()
            .zip(targets)
            .filter(|(p, _)| !ambiguous.contains(p))
            .unzip();
        let aliases = (!patterns.is_empty())
            .then(|| {
                AhoCorasick::builder()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(&patterns)
                    .ok()
            })
            .flatten();

        let mut keys: HashMap<String, (usize, usize)> = HashMap::new();
        let mut shared: HashSet<String> = HashSet::new();
        let mut all_keys = HashSet::new();
        for (i, entry) in entries.iter().enumerate() {
            let forms = std::iter::once(&entry.written)
                .chain(&entry.spoken_forms)
                .chain(&entry.misrecognitions);
            for form in forms {
                let key = form_key(form);
                if key.is_empty() {
                    continue;
                }
                all_keys.insert(key.text.clone());
                if !correctable(entry) {
                    continue;
                }
                match keys.get(&key.text) {
                    Some(&(other, _)) if entries[other].written != entry.written => {
                        shared.insert(key.text.clone());
                    }
                    Some(_) => {}
                    None => {
                        keys.insert(key.text.clone(), (i, key.len()));
                    }
                }
            }
        }
        for key in shared {
            keys.remove(&key);
        }
        let written = entries.iter().map(|e| e.written.clone()).collect();
        let cues = entries
            .iter()
            .map(|e| {
                e.context_cues
                    .iter()
                    .map(|c| phrase(c))
                    .filter(|c| !c.is_empty())
                    .collect()
            })
            .collect();

        CorrectionIndex {
            rows,
            aliases,
            alias_targets: targets,
            keys,
            all_keys,
            written,
            cues,
        }
    }
}

impl Corrector {
    /// Builds the automaton, the key map and the fuzzy set for `eligible` as `plan` ranked
    /// them. It takes the [`EligibleEntries`] the plan was made from, never raw rows, so
    /// correction reads exactly the set the prompt and the bonus read (DD-36).
    pub fn new(eligible: &EligibleEntries, plan: &RequestPlan) -> Corrector {
        Corrector::with_index(Arc::new(CorrectionIndex::new(eligible)), plan)
    }

    /// [`Corrector::new`] over a slice, for this crate's tests.
    #[cfg(test)]
    pub(crate) fn over(entries: &[RecognitionEntry], plan: &RequestPlan) -> Corrector {
        Corrector::with_index(Arc::new(CorrectionIndex::over(entries)), plan)
    }

    /// The corrector of one request whose `plan` was made from the rows `index` was built
    /// from: only the request's own part is built here, at most 30 fuzzy keys and their
    /// sound codes. The caller pairs the two; a plan entry past the index's rows is left
    /// out, never read.
    pub fn with_index(index: Arc<CorrectionIndex>, plan: &RequestPlan) -> Corrector {
        let entries = index.rows.get();
        let fuzzy = plan
            .fuzzy_set
            .iter()
            .filter_map(|&i| {
                let entry = entries.get(i)?;
                let key = form_key(&entry.written).text;
                let sound = double_metaphone(&key);
                Some((i, key, sound))
            })
            .collect();
        let in_context = (0..entries.len())
            .map(|i| plan.in_context.get(i).copied().unwrap_or(false))
            .collect();
        Corrector {
            index,
            in_context,
            fuzzy,
        }
    }

    fn entries(&self) -> &[RecognitionEntry] {
        self.index.rows.get()
    }

    /// Corrects `text`. `prompt_sent` is the prompt's terms as the worker fitted them,
    /// for the fuzzy matcher's prompt piece guard.
    pub fn correct(&self, text: &str, prompt_sent: &[String]) -> Corrected {
        let text: String = text.nfc().collect();
        if self.entries().is_empty() || text.trim().is_empty() {
            return Corrected {
                text,
                changes: Vec::new(),
            };
        }
        let utterance = utterance_phrases(&text);
        let licensed: Vec<bool> = (0..self.entries().len())
            .map(|i| {
                self.in_context.get(i).copied().unwrap_or(false)
                    || self
                        .index
                        .cues
                        .get(i)
                        .is_some_and(|cues| cues.iter().any(|c| utterance.contains(c)))
            })
            .collect();
        let prompt_folded: Vec<(String, String)> = prompt_sent
            .iter()
            .map(|t| (t.to_lowercase(), form_key(t).text))
            .collect();

        let mut doc = Document::new(&text);
        doc.apply(Stage::Alias, |piece, before| {
            self.alias_edits(piece, before, &licensed)
        });
        doc.apply(Stage::Exact, |piece, before| {
            self.exact_edits(piece, before, &licensed)
        });
        doc.apply(Stage::Fuzzy, |piece, before| {
            self.fuzzy_edits(piece, before, &licensed, &prompt_folded)
        });
        doc.finish(self.entries())
    }

    fn alias_edits(&self, piece: &str, _before: Option<char>, licensed: &[bool]) -> Vec<Edit> {
        let Some(automaton) = &self.index.aliases else {
            return Vec::new();
        };
        let folded = Folded::new(piece);
        let mut edits = Vec::new();
        for found in automaton.find_iter(folded.text.as_str()) {
            let Some(&(entry, wordy)) = self.index.alias_targets.get(found.pattern().as_usize())
            else {
                continue;
            };
            let Some(range) = folded.original(found.start(), found.end()) else {
                continue;
            };
            if !whole_word(piece, &range)
                || (wordy && !licensed.get(entry).copied().unwrap_or(false))
            {
                continue;
            }
            let Some(from) = piece.get(range.clone()) else {
                continue;
            };
            if from == self.entries()[entry].written {
                continue;
            }
            edits.push(Edit { range, entry });
        }
        edits
    }

    fn exact_edits(&self, piece: &str, before: Option<char>, licensed: &[bool]) -> Vec<Edit> {
        let mut edits = Vec::new();
        for segment in segments(piece, before) {
            let mut p = 0;
            while p < segment.len() {
                let mut advance = 1;
                for w in (1..=MAX_WINDOW.min(segment.len() - p)).rev() {
                    let window = &segment[p..p + w];
                    let texts: Vec<&str> = window.iter().map(|t| t.text).collect();
                    let key = window_key(&texts);
                    let Some(&(entry, form_len)) = self.index.keys.get(&key.text) else {
                        continue;
                    };
                    if w > MAX_WINDOW.min(form_len + 1) {
                        continue;
                    }
                    let range = window[0].range.start..window[w - 1].range.end;
                    let span = piece.get(range.clone()).unwrap_or_default();
                    if self.index.written.contains(span) {
                        // Already a term (this one or another): nothing inside it changes.
                        advance = w;
                        break;
                    }
                    if !self.exact_allowed(entry, window, &key, licensed) {
                        continue;
                    }
                    edits.push(Edit { range, entry });
                    advance = w;
                    break;
                }
                p += advance;
            }
        }
        edits
    }

    /// The exact key matcher's guards (DD-49).
    fn exact_allowed(&self, entry: usize, window: &[Token], key: &Key, licensed: &[bool]) -> bool {
        let target = &self.entries()[entry];
        // `PhD two` stays: a digits_literal entry never takes digits from number words.
        if target.digits_literal && key.from_number_words {
            return false;
        }
        if let [token] = window {
            // `spoke` stays; mid sentence `Spoke` becomes SPOKE.
            if words::is_english(token.text) {
                let mut chars = token.text.chars();
                let first_upper = chars.next().is_some_and(char::is_uppercase);
                let inner_upper = chars.any(char::is_uppercase);
                if !(inner_upper || (first_upper && !token.sentence_start)) {
                    return false;
                }
            }
            return true;
        }
        let texts: Vec<&str> = window.iter().map(|t| t.text).collect();
        !is_wordy(&texts) || licensed.get(entry).copied().unwrap_or(false)
    }

    fn fuzzy_edits(
        &self,
        piece: &str,
        before: Option<char>,
        licensed: &[bool],
        prompt: &[(String, String)],
    ) -> Vec<Edit> {
        if self.fuzzy.is_empty() {
            return Vec::new();
        }
        let mut edits = Vec::new();
        for segment in segments(piece, before) {
            let mut p = 0;
            while p < segment.len() {
                let mut best: Option<(f64, usize, usize)> = None; // (similarity, width, entry)
                for w in 1..=MAX_FUZZY_SPAN.min(segment.len() - p) {
                    let span = &segment[p..p + w];
                    if let Some((similarity, entry)) = self.fuzzy_span(span, licensed, prompt) {
                        if best.is_none_or(|(s, _, _)| similarity > s) {
                            best = Some((similarity, w, entry));
                        }
                    }
                }
                match best {
                    Some((_, w, entry)) => {
                        let range = segment[p].range.start..segment[p + w - 1].range.end;
                        edits.push(Edit { range, entry });
                        p += w;
                    }
                    None => p += 1,
                }
            }
        }
        edits
    }

    /// The best fuzzy target of one span, after every guard, or `None`.
    fn fuzzy_span(
        &self,
        span: &[Token],
        licensed: &[bool],
        prompt: &[(String, String)],
    ) -> Option<(f64, usize)> {
        let texts: Vec<&str> = span.iter().map(|t| t.text).collect();
        if span.len() > 1 {
            let edge_stop = |t: &&str| STOP_WORDS.contains(&t.to_lowercase().as_str());
            if texts.first().is_some_and(edge_stop) || texts.last().is_some_and(edge_stop) {
                return None;
            }
        }
        let key = window_key(&texts).text;
        if key.chars().count() < MIN_FUZZY_KEY || self.index.all_keys.contains(&key) {
            return None;
        }
        let english = texts.iter().any(|t| words::is_english(t));
        let span_folded = texts.join(" ").to_lowercase();
        let digits = |k: &str| -> String { k.chars().filter(char::is_ascii_digit).collect() };
        let sound = double_metaphone(&key);
        let mut scored: Vec<(f64, usize)> = Vec::new();
        for (entry, entry_key, entry_sound) in &self.fuzzy {
            let ratio = key.chars().count() as f64 / entry_key.chars().count().max(1) as f64;
            if !(0.67..=1.5).contains(&ratio) || digits(&key) != digits(entry_key) {
                continue;
            }
            let similarity = strsim::normalized_levenshtein(&key, entry_key);
            let passes = similarity >= FUZZY_SIMILARITY
                || (similarity >= FUZZY_SIMILARITY_WITH_SOUND
                    && sound_similarity(&sound, entry_sound) >= SOUND_SIMILARITY);
            if !passes {
                continue;
            }
            scored.push((similarity, *entry));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (best, entry) = *scored.first()?;
        if scored.get(1).is_some_and(|(runner_up, other)| {
            best - runner_up < ABSTAIN_WITHIN
                && self.entries()[*other].written != self.entries()[entry].written
        }) {
            return None;
        }
        let licensed_here = licensed.get(entry).copied().unwrap_or(false);
        if english && !licensed_here {
            return None;
        }
        let target_key = form_key(&self.entries()[entry].written).text;
        if STOP_PAIRS
            .iter()
            .any(|(a, b)| (key == *a && target_key == *b) || (key == *b && target_key == *a))
        {
            return None;
        }
        // A truncated piece of another entry that was in the prompt (`Moduli` of
        // `Modulite`) is the prompt leaking, not a misheard term.
        let target_written = self.entries()[entry].written.to_lowercase();
        if span_folded.chars().count() >= MIN_PROMPT_PIECE
            && prompt.iter().any(|(term, term_key)| {
                *term != target_written
                    && (term.starts_with(&span_folded) || term_key.starts_with(&key))
                    && *term != span_folded
            })
        {
            return None;
        }
        if texts.join(" ") == self.entries()[entry].written {
            return None;
        }
        Some((best, entry))
    }
}

/// Whether every token is an English word: a small single letter counts as a word (`a`,
/// `i`); a capital single letter is a spelled letter, and a token with a digit is not a
/// word.
fn is_wordy<S: AsRef<str>>(tokens: &[S]) -> bool {
    !tokens.is_empty()
        && tokens.iter().all(|t| {
            let t = t.as_ref();
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_uppercase() => false,
                _ => !t.chars().any(|c| c.is_ascii_digit()) && words::is_english(t),
            }
        })
}

/// A cue or a phrase as the utterance check compares them: lower case words, one space.
fn phrase(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every run of 1 to 6 words of the utterance, for the cue check.
fn utterance_phrases(text: &str) -> HashSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut phrases = HashSet::new();
    for start in 0..words.len() {
        for end in start + 1..=(start + 6).min(words.len()) {
            phrases.insert(words[start..end].join(" "));
        }
    }
    phrases
}

/// Whether `range` of `text` stands as whole words: no letter, digit or `_` touches it.
fn whole_word(text: &str, range: &std::ops::Range<usize>) -> bool {
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    let before = text.get(..range.start).and_then(|s| s.chars().next_back());
    let after = text.get(range.end..).and_then(|s| s.chars().next());
    !before.is_some_and(word_char) && !after.is_some_and(word_char)
}

/// A piece's text folded to lower case, with the original byte offset of each folded byte.
struct Folded {
    text: String,
    /// For each folded byte, the original byte offset of the character it came from, and
    /// whether it is the first folded byte of that character.
    origin: Vec<(usize, bool)>,
    original_len: usize,
}

impl Folded {
    fn new(original: &str) -> Folded {
        let mut text = String::with_capacity(original.len());
        let mut origin = Vec::with_capacity(original.len());
        for (at, c) in original.char_indices() {
            let mut first = true;
            for lower in c.to_lowercase() {
                let mut buf = [0u8; 4];
                for _ in lower.encode_utf8(&mut buf).bytes() {
                    origin.push((at, first));
                    first = false;
                }
                text.push(lower);
            }
        }
        Folded {
            text,
            origin,
            original_len: original.len(),
        }
    }

    /// The original byte range of a folded match, when it starts and ends on whole
    /// original characters.
    fn original(&self, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
        let (from, first) = *self.origin.get(start)?;
        if !first {
            return None;
        }
        let to = match self.origin.get(end) {
            Some((at, true)) => *at,
            Some(_) => return None,
            None => self.original_len,
        };
        Some(from..to)
    }
}

/// One token of a free piece: its text and range, and whether a sentence starts with it.
#[derive(Clone, Debug)]
struct Token<'a> {
    text: &'a str,
    range: std::ops::Range<usize>,
    sentence_start: bool,
}

/// The whitespace separated chunks of `text`: each chunk's byte range, and the range of its
/// core, the chunk without the punctuation at its edges.
fn chunks(text: &str) -> Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut at = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let start = at;
        at += chunk.len();
        let word = chunk.trim_end_matches(char::is_whitespace);
        if word.is_empty() {
            continue;
        }
        let lead = word.len() - word.trim_start_matches(EDGE).len();
        let core = word.trim_start_matches(EDGE).trim_end_matches(EDGE);
        out.push((
            start..start + word.len(),
            start + lead..start + lead + core.len(),
        ));
    }
    out
}

/// Whether a token's core is never changed (step 1).
fn is_protected(core: &str) -> bool {
    core.contains(PROTECTING)
        || core.contains("://")
        || core.to_lowercase().starts_with("www.")
        || core.contains('.')
}

/// Splits `text` into free pieces and protected pieces, one per protected token core.
fn protect_tokens(text: &str, pieces: &mut Vec<Piece>) {
    let mut cursor = 0;
    for (_, core) in chunks(text) {
        let Some(word) = text.get(core.clone()) else {
            continue;
        };
        if word.is_empty() || !is_protected(word) {
            continue;
        }
        if let Some(before) = text.get(cursor..core.start).filter(|s| !s.is_empty()) {
            pieces.push(Piece::Free(before.to_string()));
        }
        pieces.push(Piece::Protected(word.to_string()));
        cursor = core.end;
    }
    if let Some(rest) = text.get(cursor..).filter(|s| !s.is_empty()) {
        pieces.push(Piece::Free(rest.to_string()));
    }
}

/// The tokens of `piece` in runs no punctuation breaks. Tokens are split at whitespace and
/// hyphens; punctuation at a token's edge ends the run, as would a protected token, which
/// step 1 already moved into a piece of its own. `before` is the last character before
/// the piece that is not whitespace.
fn segments(piece: &str, before: Option<char>) -> Vec<Vec<Token<'_>>> {
    let mut runs: Vec<Vec<Token<'_>>> = Vec::new();
    let mut run: Vec<Token<'_>> = Vec::new();
    let mut sentence_start = before.is_none_or(|c| matches!(c, '.' | '!' | '?' | '\n'));
    for (word, core_range) in chunks(piece) {
        let start = word.start;
        let core = piece.get(core_range.clone()).unwrap_or_default();
        let lead = core_range.start - word.start;
        let trail = word.end - core_range.end;
        let trailing = piece.get(core_range.end..word.end).unwrap_or_default();
        if lead > 0 {
            close(&mut runs, &mut run);
        }
        if is_protected(core) || core.is_empty() {
            close(&mut runs, &mut run);
        } else {
            let core_start = start + lead;
            let mut offset = 0;
            for part in core.split_inclusive(['-', '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}'])
            {
                let text =
                    part.trim_end_matches(['-', '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}']);
                if !text.is_empty() {
                    let s = core_start + offset;
                    run.push(Token {
                        text,
                        range: s..s + text.len(),
                        sentence_start,
                    });
                    sentence_start = false;
                }
                offset += part.len();
            }
        }
        if trail > 0 {
            if trailing.contains(['.', '!', '?']) {
                sentence_start = true;
            }
            close(&mut runs, &mut run);
        }
        if piece.get(word.end..).is_some_and(|rest| {
            rest.chars()
                .take_while(|c| c.is_whitespace())
                .any(|c| c == '\n')
        }) {
            sentence_start = true;
        }
    }
    close(&mut runs, &mut run);
    runs
}

fn close<'a>(runs: &mut Vec<Vec<Token<'a>>>, run: &mut Vec<Token<'a>>) {
    if !run.is_empty() {
        runs.push(std::mem::take(run));
    }
}

/// One replacement inside a piece.
struct Edit {
    range: std::ops::Range<usize>,
    entry: usize,
}

enum Piece {
    /// Text a later step may change.
    Free(String),
    /// Text that never changes: backtick spans.
    Protected(String),
    /// A change: what was there, the entry written there, and the step.
    Changed {
        from: String,
        entry: usize,
        stage: Stage,
    },
}

/// The text as pieces, so a span one step changed is never changed again and change
/// offsets are computed once, at the end.
struct Document {
    pieces: Vec<Piece>,
}

impl Document {
    /// Step 1: backtick spans, and every token that holds a protecting character, an
    /// inner `.`, a URL scheme or a `www.` start, become protected pieces, which no step
    /// reads and no window crosses.
    fn new(text: &str) -> Document {
        let mut pieces = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find('`') {
            let after = rest.get(open + 1..).unwrap_or_default();
            let Some(close) = after.find('`') else {
                break;
            };
            if open > 0 {
                protect_tokens(rest.get(..open).unwrap_or_default(), &mut pieces);
            }
            // Both backticks are one byte, so the span ends `close + 2` bytes after `open`.
            let span = rest.get(open..open + close + 2).unwrap_or_default();
            pieces.push(Piece::Protected(span.to_string()));
            rest = after.get(close + 1..).unwrap_or_default();
        }
        if !rest.is_empty() {
            protect_tokens(rest, &mut pieces);
        }
        Document { pieces }
    }

    /// Runs one step over every free piece. `edits` returns non-overlapping ranges in
    /// order; anything else it returns is dropped.
    fn apply(&mut self, stage: Stage, mut edits: impl FnMut(&str, Option<char>) -> Vec<Edit>) {
        let mut out = Vec::with_capacity(self.pieces.len());
        let mut before: Option<char> = None;
        for piece in std::mem::take(&mut self.pieces) {
            match piece {
                Piece::Free(text) => {
                    let mut cursor = 0;
                    for edit in edits(&text, before) {
                        let (Some(gap), Some(from)) = (
                            text.get(cursor..edit.range.start),
                            text.get(edit.range.clone()),
                        ) else {
                            // Overlapping or not on character boundaries: dropped.
                            continue;
                        };
                        if !gap.is_empty() {
                            out.push(Piece::Free(gap.to_string()));
                        }
                        out.push(Piece::Changed {
                            from: from.to_string(),
                            entry: edit.entry,
                            stage,
                        });
                        cursor = edit.range.end;
                    }
                    if let Some(rest) = text.get(cursor..).filter(|s| !s.is_empty()) {
                        out.push(Piece::Free(rest.to_string()));
                    }
                    before = text.trim_end().chars().next_back().or(before);
                }
                Piece::Protected(text) => {
                    before = text.chars().next_back();
                    out.push(Piece::Protected(text));
                }
                changed @ Piece::Changed { .. } => {
                    before = Some('x');
                    out.push(changed);
                }
            }
        }
        // A step's pieces are re-merged so the next step reads whole runs of free text.
        let mut merged: Vec<Piece> = Vec::with_capacity(out.len());
        for piece in out {
            match (merged.last_mut(), piece) {
                (Some(Piece::Free(last)), Piece::Free(next)) => last.push_str(&next),
                (_, piece) => merged.push(piece),
            }
        }
        self.pieces = merged;
    }

    fn finish(self, entries: &[RecognitionEntry]) -> Corrected {
        let mut text = String::new();
        let mut chars = 0usize;
        let mut changes = Vec::new();
        for piece in self.pieces {
            match piece {
                Piece::Free(t) | Piece::Protected(t) => {
                    chars += t.chars().count();
                    text.push_str(&t);
                }
                Piece::Changed { from, entry, stage } => {
                    let Some(target) = entries.get(entry) else {
                        chars += from.chars().count();
                        text.push_str(&from);
                        continue;
                    };
                    let len = target.written.chars().count();
                    changes.push(Change {
                        start: chars,
                        end: chars + len,
                        from,
                        to: target.written.clone(),
                        stage,
                        entry_id: target.id.clone(),
                    });
                    chars += len;
                    text.push_str(&target.written);
                }
            }
        }
        Corrected { text, changes }
    }
}

/// Double Metaphone codes of a key, primary and alternate: a compact port of Lawrence
/// Philips's algorithm for the consonant classes English spelling confuses most (C, G, K,
/// Q and X; CH, SH and TH; PH and V to F; silent initial letters; vowels only at the
/// start). Codes are at most 6 characters. Digits and anything not a Latin letter are
/// left out.
pub fn double_metaphone(key: &str) -> (String, String) {
    let w: Vec<char> = key
        .chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let mut primary = String::new();
    let mut alternate = String::new();
    let at = |i: usize| w.get(i).copied().unwrap_or('\0');
    let vowel = |c: char| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U' | 'Y');
    let mut push = |p: &str, a: &str| {
        primary.push_str(p);
        alternate.push_str(a);
    };
    let mut i = 0;
    if matches!(
        (at(0), at(1)),
        ('G', 'N') | ('K', 'N') | ('P', 'N') | ('W', 'R') | ('P', 'S')
    ) {
        i = 1;
    }
    if at(0) == 'X' {
        push("S", "S");
        i = 1;
    }
    while i < w.len() {
        let c = at(i);
        let next = at(i + 1);
        let mut step = 1;
        match c {
            _ if vowel(c) => {
                if i == 0 {
                    push("A", "A");
                }
            }
            'B' => {
                push("P", "P");
                if next == 'B' {
                    step = 2;
                }
            }
            'C' => {
                if next == 'H' {
                    if matches!(at(i + 2), 'R' | 'L') || (i == 0 && !vowel(at(i + 2))) {
                        push("K", "K");
                    } else {
                        push("X", "K");
                    }
                    step = 2;
                } else if next == 'I' && at(i + 2) == 'A' {
                    push("X", "X");
                    step = 3;
                } else if matches!(next, 'E' | 'I' | 'Y') {
                    push("S", "S");
                    step = 2;
                } else if next == 'C' && matches!(at(i + 2), 'E' | 'I' | 'Y') {
                    push("KS", "KS");
                    step = 3;
                } else {
                    push("K", "K");
                    if matches!(next, 'K' | 'Q' | 'C') {
                        step = 2;
                    }
                }
            }
            'D' => {
                if next == 'G' && matches!(at(i + 2), 'E' | 'I' | 'Y') {
                    push("J", "J");
                    step = 3;
                } else {
                    push("T", "T");
                    if matches!(next, 'T' | 'D') {
                        step = 2;
                    }
                }
            }
            'F' | 'V' => {
                push("F", "F");
                if matches!(next, 'F' | 'V') {
                    step = 2;
                }
            }
            'G' => {
                if next == 'H' {
                    if i == 0 {
                        push("K", "K");
                    }
                    step = 2;
                } else if next == 'N' {
                    push("N", "KN");
                    step = 2;
                } else if matches!(next, 'E' | 'I' | 'Y') {
                    push("J", "K");
                    step = 2;
                } else {
                    push("K", "K");
                    if next == 'G' {
                        step = 2;
                    }
                }
            }
            'H' => {
                if (i == 0 || vowel(at(i.wrapping_sub(1)))) && vowel(next) {
                    push("H", "H");
                    step = 2;
                }
            }
            'J' => push("J", "H"),
            'K' | 'Q' => {
                push("K", "K");
                if matches!(next, 'K' | 'Q') {
                    step = 2;
                }
            }
            'L' | 'M' | 'N' | 'R' => {
                let s = c.to_string();
                push(&s, &s);
                if next == c {
                    step = 2;
                }
            }
            'P' => {
                if next == 'H' {
                    push("F", "F");
                    step = 2;
                } else {
                    push("P", "P");
                    if matches!(next, 'P' | 'B') {
                        step = 2;
                    }
                }
            }
            'S' => {
                if next == 'H' {
                    push("X", "X");
                    step = 2;
                } else if next == 'C' && at(i + 2) == 'H' {
                    push("SK", "SK");
                    step = 3;
                } else if next == 'I' && matches!(at(i + 2), 'O' | 'A') {
                    push("X", "S");
                    step = 3;
                } else {
                    push("S", "S");
                    if matches!(next, 'S' | 'Z') {
                        step = 2;
                    }
                }
            }
            'T' => {
                if next == 'H' {
                    push("0", "T");
                    step = 2;
                } else if next == 'I' && matches!(at(i + 2), 'O' | 'A') {
                    push("X", "X");
                    step = 3;
                } else if next == 'C' && at(i + 2) == 'H' {
                    step = 1;
                } else {
                    push("T", "T");
                    if matches!(next, 'T' | 'D') {
                        step = 2;
                    }
                }
            }
            'W' => {
                if vowel(next) {
                    push("W", "F");
                }
            }
            'X' => push("KS", "KS"),
            'Z' => {
                push("S", "TS");
                if next == 'Z' {
                    step = 2;
                }
            }
            _ => {}
        }
        i += step;
    }
    primary.truncate(6);
    alternate.truncate(6);
    (primary, alternate)
}

/// How alike two words sound: the best normalized Levenshtein similarity over their
/// primary and alternate codes.
pub fn sound_similarity(a: &(String, String), b: &(String, String)) -> f64 {
    [(&a.0, &b.0), (&a.0, &b.1), (&a.1, &b.0), (&a.1, &b.1)]
        .iter()
        .filter(|(x, y)| !x.is_empty() && !y.is_empty())
        .map(|(x, y)| strsim::normalized_levenshtein(x, y))
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::super::select::plan_entries;
    use super::super::test_entries::{entry, with};
    use super::*;
    use crate::store::TermKind;

    fn dictionary() -> Vec<RecognitionEntry> {
        vec![
            with(entry("Ventolin"), |e| e.kind = Some(TermKind::Name)),
            with(entry("Clenil"), |e| e.kind = Some(TermKind::Name)),
            with(entry("FLT3"), |e| {
                e.kind = Some(TermKind::GeneSymbol);
                e.misrecognitions = vec!["FLET3".into(), "FLAT3".into()];
            }),
            with(entry("ALOX5AP"), |e| {
                e.kind = Some(TermKind::GeneSymbol);
                e.misrecognitions = vec!["ALAS5AP".into(), "ALAX5AP".into()];
            }),
            with(entry("PROKR2"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("IGF1R"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("EPAS1"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("PHD2"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("HRAS"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("ARNT"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("NRAS"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("NTRK"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("UCSF"), |e| e.kind = Some(TermKind::Acronym)),
            with(entry("EGLN1"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("SPOKE"), |e| e.kind = Some(TermKind::Acronym)),
            with(entry("CRISPRi"), |e| e.kind = Some(TermKind::Term)),
            with(entry("codeine"), |e| e.kind = Some(TermKind::Term)),
            with(entry("OMOP"), |e| {
                e.kind = Some(TermKind::Acronym);
                e.spoken_forms = vec!["oh mop".into()];
            }),
            with(entry("Biorouter"), |e| {
                e.kind = Some(TermKind::Name);
                e.spoken_forms = vec!["bio router".into()];
            }),
            with(entry("ENCODE"), |e| e.kind = Some(TermKind::Acronym)),
            with(entry("CELSR1"), |e| e.kind = Some(TermKind::GeneSymbol)),
            with(entry("Bostonite"), |e| e.kind = Some(TermKind::Term)),
        ]
    }

    fn run(text: &str, context: &str) -> Corrected {
        let entries = dictionary();
        let plan = plan_entries(&entries, context, chrono::Utc::now());
        Corrector::over(&entries, &plan).correct(text, &plan.prompt_terms)
    }

    /// PERF-1: one [`CorrectionIndex`] shared by requests of different contexts corrects
    /// every text as a corrector built whole for each request does, through each step and
    /// each context licence; a plan entry past the index's rows is never read.
    #[test]
    fn a_shared_index_corrects_as_a_corrector_built_whole() {
        let entries: Vec<RecognitionEntry> = dictionary()
            .into_iter()
            .chain([
                with(entry("Modulite"), |e| {
                    e.context_cues = vec!["the rock".into()]
                }),
                with(entry("natalizumab"), |e| {
                    e.match_policy = MatchPolicy::PromptOnly
                }),
            ])
            .collect();
        let shared = Arc::new(CorrectionIndex::over(&entries));
        assert_eq!(shared.len(), entries.len());
        let texts = [
            "We looked at ventilin, klenl and FLET3 today.",
            "P-R-O-K-R2 and IGF-1R, then E-PAS-1 and pH D2.",
            "The bio router in the closet, oh mop and Spoke.",
            "Then spoke went to UCSD; CRISPR i think, and codeine.",
            "the rock held moduli and Modulit, near natalizumab.",
            "`FLET3` at https://example.org/FLET3 and a@b.c stay.",
        ];
        for context in ["", "OMOP and Biorouter", "bio router, oh mop and Modulite"] {
            let plan = plan_entries(&entries, context, chrono::Utc::now());
            let whole = Corrector::over(&entries, &plan);
            let indexed = Corrector::with_index(Arc::clone(&shared), &plan);
            for text in texts {
                assert_eq!(
                    indexed.correct(text, &plan.prompt_terms),
                    whole.correct(text, &plan.prompt_terms),
                    "{context:?}: {text:?}"
                );
            }
        }
        let mut stray = plan_entries(&entries, "", chrono::Utc::now());
        stray.fuzzy_set.push(entries.len() + 3);
        stray.in_context.push(true);
        let corrector = Corrector::with_index(Arc::clone(&shared), &stray);
        assert_eq!(corrector.in_context.len(), entries.len());
        assert!(corrector.fuzzy.iter().all(|(i, _, _)| *i < entries.len()));
    }

    /// PR-14's positives: each becomes its term, by the step that should catch it.
    #[test]
    fn the_positives_are_corrected() {
        for (said, written, stage) in [
            ("ventilin", "Ventolin", Stage::Fuzzy),
            ("klenl", "Clenil", Stage::Fuzzy),
            ("FLET3", "FLT3", Stage::Alias),
            ("ALAS5AP", "ALOX5AP", Stage::Alias),
            ("P-R-O-K-R2", "PROKR2", Stage::Exact),
            ("IGF-1R", "IGF1R", Stage::Exact),
            ("E-PAS-1", "EPAS1", Stage::Exact),
            ("pH D2", "PHD2", Stage::Exact),
        ] {
            let text = format!("We looked at {said} today.");
            let out = run(&text, "");
            assert_eq!(out.text, format!("We looked at {written} today."), "{said}");
            assert_eq!(out.changes.len(), 1, "{said}");
            let change = &out.changes[0];
            assert_eq!(
                (change.from.as_str(), change.stage),
                (said, stage),
                "{said}"
            );
            assert_eq!(change.to, written);
            let chars: Vec<char> = out.text.chars().collect();
            let at: String = chars[change.start..change.end].iter().collect();
            assert_eq!(
                at, written,
                "{said}: the offsets bound the term in the final text"
            );
        }
        // E P A S one, spelled out.
        assert_eq!(run("Check E P A S one.", "").text, "Check EPAS1.");
    }

    /// PR-14's negatives: every one stays as written.
    #[test]
    fn the_negatives_stay() {
        for text in [
            "here's the plan",
            "they aren't here",
            "the UCSD campus",
            "on exon 1 only",
            "a N-TRAC result",
            "the clinic opens",
            "continue the dose",
            "coding regions",
            "do not smoke",
            "a PhD two years ago",
            "the crisper drawer",
            "Oh mop it up",
            "The bio router in the closet",
            "CRISPR i think",
            "it encodes cells",
            "Boston is cold",
        ] {
            let out = run(text, "");
            assert_eq!(out.text, text, "{text}: {:?}", out.changes);
            assert!(out.changes.is_empty(), "{text}");
        }
    }

    /// The stop list of DD-49 alone keeps this: `clinic` is an English word licensed by
    /// Clenil being in the context, so the English guard lets it through, and the pair
    /// clears the fuzzy thresholds. Without the stop list, `the clinic opens` became `the
    /// Clenil opens`. (The list's other pairs name an entry fuzzy matching never reaches
    /// anyway: one of 5 characters or fewer, with a letter and digit shape, or that
    /// collides.)
    #[test]
    fn the_stop_list_holds_where_context_lifts_the_english_guard() {
        let out = run("the clinic opens", "Clenil inhaler");
        assert_eq!(out.text, "the clinic opens", "{:?}", out.changes);
        let (key, target) = ("clinic", "clenil");
        assert!(strsim::normalized_levenshtein(key, target) >= FUZZY_SIMILARITY_WITH_SOUND);
        assert!(
            sound_similarity(&double_metaphone(key), &double_metaphone(target)) >= SOUND_SIMILARITY
        );
        // A misheard form the stop list does not name is still corrected in that context.
        assert_eq!(
            run("the klenl opens", "Clenil inhaler").text,
            "the Clenil opens"
        );
    }

    /// DD-49's abstain rule: when two different entries match a span within 0.04 of each
    /// other, the span is left alone; with one of them, or with a clear winner, it is
    /// corrected.
    #[test]
    fn two_close_fuzzy_targets_abstain() {
        let named = |written: &str| with(entry(written), |e| e.kind = Some(TermKind::Name));
        let correct = |entries: Vec<RecognitionEntry>, text: &str| -> String {
            let plan = plan_entries(&entries, "", chrono::Utc::now());
            assert_eq!(plan.fuzzy_set.len(), entries.len(), "all fuzzy capable");
            Corrector::over(&entries, &plan).correct(text, &[]).text
        };
        // `zorvatan` is one edit from each: 0.875 and 0.875.
        assert_eq!(
            correct(vec![named("Zorvatin")], "take Zorvatan daily"),
            "take Zorvatin daily"
        );
        assert_eq!(
            correct(
                vec![named("Zorvatin"), named("Zorvaton")],
                "take Zorvatan daily"
            ),
            "take Zorvatan daily"
        );
        // `zorvatinn` is 0.889 from Zorvatin and 0.778 from Zorvaton: a clear winner.
        assert_eq!(
            correct(
                vec![named("Zorvatin"), named("Zorvaton")],
                "take Zorvatinn daily"
            ),
            "take Zorvatin daily"
        );
    }

    #[test]
    fn wordy_forms_apply_in_context() {
        let out = run("The bio router in the closet", "notes about Biorouter");
        assert_eq!(out.text, "The Biorouter in the closet");
        assert_eq!(out.changes[0].stage, Stage::Alias);
        assert_eq!(run("Oh mop it up", "the OMOP tables").text, "OMOP it up");
    }

    #[test]
    fn single_english_tokens_need_a_capital_mid_sentence() {
        assert_eq!(run("We saw spoke data.", "").text, "We saw spoke data.");
        assert_eq!(run("We saw Spoke data.", "").text, "We saw SPOKE data.");
        assert_eq!(run("Spoke data is here.", "").text, "Spoke data is here.");
    }

    #[test]
    fn protected_spans_never_change() {
        for text in [
            "run `E-PAS-1` now",
            "see https://e-pas-1.example now",
            "mail e-pas-1@example.org now",
            "open ./E-PAS-1/x now",
            "HLA-B*57:01 and E_PAS_1",
        ] {
            assert_eq!(run(text, "").text, text, "{text}");
        }
        assert_eq!(
            run("run `x` then E-PAS-1", "").text,
            "run `x` then EPAS1",
            "the text after a protected span is still corrected"
        );
    }

    #[test]
    fn windows_never_cross_punctuation() {
        assert_eq!(run("E-PAS, one", "").text, "E-PAS, one");
        assert_eq!(run("FLET3. and FLET3", "").text, "FLT3. and FLT3");
    }

    #[test]
    fn a_key_two_terms_share_is_skipped() {
        let entries = vec![entry("EPAS1"), entry("Epas1")];
        let plan = plan_entries(&entries, "", chrono::Utc::now());
        let out = Corrector::over(&entries, &plan).correct("E-PAS-1", &[]);
        assert_eq!(out.text, "E-PAS-1");
    }

    #[test]
    fn the_sound_codes() {
        assert_eq!(double_metaphone("klenl").0, double_metaphone("clenil").0);
        assert_eq!(
            double_metaphone("ventilin").0,
            double_metaphone("ventolin").0
        );
        assert_eq!(double_metaphone("phone").0, "FN");
        assert_eq!(double_metaphone("knight").0, "NT");
        assert!(sound_similarity(&double_metaphone("clinic"), &double_metaphone("clenil")) >= 0.75);
    }

    #[test]
    fn a_prompt_piece_is_not_corrected_into_another_term() {
        let entries = vec![
            with(entry("Modulite"), |e| e.kind = Some(TermKind::Name)),
            with(entry("Modulin"), |e| e.kind = Some(TermKind::Name)),
        ];
        let plan = plan_entries(&entries, "", chrono::Utc::now());
        let corrector = Corrector::over(&entries, &plan);
        let out = corrector.correct("piracy to Moduli today", &["Modulite".into()]);
        assert_eq!(out.text, "piracy to Moduli today");
    }

    /// The correction of one utterance at the cap (5,000 entries), which runs between the
    /// decode and the text, apart from what a stream builds once when it is created (the
    /// ranking and the corrector). PERF-1 asks for an index only if a release build spends
    /// more than about 5 ms per utterance here. A timing run, by hand, in both builds:
    /// `cargo test --release -p biorouter-dictation --lib recognize::correct::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing run, by hand"]
    fn measure_correction_at_the_cap() {
        use std::time::{Duration, Instant};
        fn time<T>(runs: usize, mut work: impl FnMut() -> T) -> Duration {
            let mut samples: Vec<Duration> = (0..runs)
                .map(|_| {
                    let started = Instant::now();
                    std::hint::black_box(work());
                    started.elapsed()
                })
                .collect();
            samples.sort();
            samples[samples.len() / 2]
        }
        crate::recognize::words::warm();
        for forms in [false, true] {
            let entries: Vec<RecognitionEntry> = (0..crate::store::ops::MAX_ACTIVE_PER_TIER)
                .map(|i| {
                    with(entry(&format!("BRL{i:04}X")), |e| {
                        if forms {
                            e.spoken_forms = vec![format!("barrel {i} spoken")];
                            e.misrecognitions = vec![format!("barrel {i} heard")];
                        }
                    })
                })
                .collect();
            let plan = time(9, || plan_entries(&entries, "", chrono::Utc::now()));
            let ranked = plan_entries(&entries, "", chrono::Utc::now());
            let build = time(9, || Corrector::over(&entries, &ranked));
            // PERF-1: what a daemon builds once per set, and what each request then adds.
            let rank_index = time(9, || super::super::select::RankIndex::over(&entries));
            let ranks = super::super::select::RankIndex::over(&entries);
            let context = "We talked about the Baranzini group, VHL and barrel 12 spoken.";
            let plan_indexed = time(51, || {
                super::super::select::plan_indexed(&entries, Some(&ranks), "", chrono::Utc::now())
            });
            let plan_indexed_context = time(51, || {
                super::super::select::plan_indexed(
                    &entries,
                    Some(&ranks),
                    context,
                    chrono::Utc::now(),
                )
            });
            let index = time(9, || CorrectionIndex::over(&entries));
            let shared = Arc::new(CorrectionIndex::over(&entries));
            let request = time(201, || Corrector::with_index(Arc::clone(&shared), &ranked));
            let corrector = Corrector::over(&entries, &ranked);
            let prompt = ranked.prompt_terms.clone();
            let plain =
                "The Baranzini group uses spoke to link VHL mutations to disease phenotypes.";
            let terms = "Then barrel 12 heard, BRL0040X and barrel 7 spoken went to the core.";
            let minute: String = std::iter::repeat_n(plain, 12).collect::<Vec<_>>().join(" ");
            let correct_plain = time(201, || corrector.correct(plain, &prompt));
            let correct_terms = time(201, || corrector.correct(terms, &prompt));
            let correct_minute = time(51, || corrector.correct(&minute, &prompt));
            println!(
                "{} entries ({}): plan {plan:?}; corrector {build:?}; once per set: rank index \
                 {rank_index:?}, correction index {index:?}; per request with them: plan \
                 {plan_indexed:?} ({plan_indexed_context:?} with context), corrector \
                 {request:?}; correct a 5 s utterance {correct_plain:?}, one with terms \
                 {correct_terms:?} ({:?}), a 60 s dictation {correct_minute:?}",
                entries.len(),
                if forms {
                    "a spoken form and a misrecognition each"
                } else {
                    "bare"
                },
                corrector.correct(terms, &prompt).text
            );
        }
    }
}
