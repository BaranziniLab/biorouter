//! What happens to the engine's words before a person sees them (DD-48, DD-49).
//!
//! The worker writes what Whisper decoded; the parent decides what of it is speech, and
//! the dictionary decides how it is spelled.
//!
//! * [`text`] joins the pieces and holds the whole output filter, which empties an output
//!   made only of what Whisper writes over silence and noise.
//! * [`echo`] holds the checks against the exact prompt that was sent (a whole echo, a
//!   trailing recital) and the collapse check, and decides whether one retry is worth
//!   making.
//! * [`words`] is the embedded English word list.
//! * [`key`] gives the normalized keys the exact matcher compares.
//! * [`select`] ranks the eligible entries against a request's context (DD-45).
//! * [`prompt`] picks the prompt's written forms and fits them to the token budget.
//! * [`correct`] rewrites the decoded text: aliases, exact keys, guarded fuzzy matching.
//!
//! Every one of those reads one value, [`EligibleEntries`]: the entries this dictation
//! may use. Its constructor is called from exactly one place,
//! `crates/biorouter/src/dictation/bias.rs`, which applies the tier and institution rules
//! (DD-36); a repository walk test there pins it.

pub mod correct;
pub mod echo;
pub mod key;
pub mod prompt;
pub mod select;
pub mod text;
pub mod words;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::protocol::{BiasEntry, BonusParams, SetBias};
use crate::store::{BiasPolicy, Entry, EntryStatus, EntryTier, MatchPolicy, TermKind, UsageRecord};

/// Why a decode was made a second time, as the transcript's `retried.reason` reports it.
/// A collapse and a deleted start are one check (fewer words than the detected speech
/// can hold), so they share one reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    Echo,
    Collapse,
}

impl RetryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RetryReason::Echo => "echo",
            RetryReason::Collapse => "collapse",
        }
    }
}

/// What a transcript reports as `bias_scope` (DD-29): `private` only when private
/// entries may have shaped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiasScope {
    Public,
    Private,
}

impl BiasScope {
    pub fn as_str(self) -> &'static str {
        match self {
            BiasScope::Public => "public",
            BiasScope::Private => "private",
        }
    }
}

/// Which decoder bonus table a set uses (oracle rule 9): the public rows, every row, or
/// the private rows one target affiliation admits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BonusTable {
    PublicRows,
    AllRows,
    /// The key of the target's affiliation, as `bias.rs` writes it.
    PrivateRows(String),
}

impl BonusTable {
    /// The `scope` of the worker's `set_bias` and `transcribe` frames.
    pub fn scope(&self) -> String {
        match self {
            BonusTable::PublicRows => "public".to_string(),
            BonusTable::AllRows => "all".to_string(),
            BonusTable::PrivateRows(key) => format!("private:{key}"),
        }
    }

    /// Which usage map ranking reads (oracle rule 11): `public` for the public rows table,
    /// `all` otherwise.
    pub fn usage_scope(&self) -> crate::store::UsageScope {
        match self {
            BonusTable::PublicRows => crate::store::UsageScope::Public,
            _ => crate::store::UsageScope::All,
        }
    }
}

/// One eligible dictionary row, as recognition reads it: twins merged, only the
/// misrecognitions that correction may use, the usage of the map the table ranks by, and
/// the flags derived from the word list.
#[derive(Clone, Debug, PartialEq)]
pub struct RecognitionEntry {
    pub id: String,
    pub tier: EntryTier,
    pub written: String,
    pub spoken_forms: Vec<String>,
    /// Misrecognitions seen 2 or more times; the rest never reach correction.
    pub misrecognitions: Vec<String>,
    pub context_cues: Vec<String>,
    pub kind: Option<TermKind>,
    pub match_policy: MatchPolicy,
    pub bias: BiasPolicy,
    pub uses: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub corrected_to: u64,
    pub updated_at: DateTime<Utc>,
    /// Derived: the written form, or its letter run before a letter and digit boundary or
    /// a case boundary, is an ordinary English word or a listed abbreviation.
    pub collides: bool,
    /// Derived: `collides` and the written form holds a digit (PHD2).
    pub digits_literal: bool,
    /// Derived: the written form holds both a letter and a digit.
    pub letter_digit: bool,
}

/// How many times a misrecognition must be seen before correction uses it.
pub const ACTIVE_MISRECOGNITION_COUNT: u32 = 2;

impl RecognitionEntry {
    /// A store row with its usage counters (`None` when it has none). The derived flags
    /// are set by [`EligibleEntries::new_filtered`]. Called only from
    /// `crates/biorouter/src/dictation/bias.rs`, after the tier and institution rules;
    /// `eligible_entries_are_built_only_by_bias_set` pins every call, and nothing public
    /// ranks or corrects a bare slice of these.
    #[doc(hidden)]
    pub fn from_entry(entry: &Entry, usage: Option<&UsageRecord>) -> RecognitionEntry {
        RecognitionEntry {
            id: entry.id.as_str().to_string(),
            tier: entry.tier,
            written: entry.written.clone(),
            spoken_forms: entry.spoken_forms.clone(),
            misrecognitions: entry
                .misrecognitions
                .iter()
                .filter(|m| m.count >= ACTIVE_MISRECOGNITION_COUNT)
                .map(|m| m.text.clone())
                .collect(),
            context_cues: entry.context_cues.clone(),
            kind: entry.kind,
            match_policy: entry.match_policy,
            bias: entry.bias,
            uses: usage.map_or(0, |u| u.uses),
            last_used_at: usage.and_then(|u| u.last_used_at),
            corrected_to: usage.map_or(0, |u| u.corrected_to),
            updated_at: entry.updated_at,
            collides: false,
            digits_literal: false,
            letter_digit: false,
        }
    }

    /// Whether a row of the store may be recognized at all: active rows only.
    pub fn is_recognizable(entry: &Entry) -> bool {
        entry.status == EntryStatus::Active
    }

    pub fn pinned(&self) -> bool {
        self.bias == BiasPolicy::Always
    }

    /// Whether the decoder bonus may start this entry (DD-46): `auto` or `always`, and not
    /// `collides` unless pinned.
    pub fn boostable(&self) -> bool {
        match self.bias {
            BiasPolicy::Never => false,
            BiasPolicy::Always => true,
            BiasPolicy::Auto => !self.collides,
        }
    }

    /// Whether fuzzy matching may ever reach this entry, whatever is stored (DD-49):
    /// `match = fuzzy`, more than 5 characters, no letter and digit shape, no collision.
    pub fn fuzzy_capable(&self) -> bool {
        self.match_policy == MatchPolicy::Fuzzy
            && self.written.chars().count() > 5
            && !self.letter_digit
            && !self.collides
    }

    fn derive_flags(&mut self) {
        let has_letter = self.written.chars().any(char::is_alphabetic);
        let has_digit = self.written.chars().any(|c| c.is_ascii_digit());
        self.letter_digit = has_letter && has_digit;
        self.collides = collides(&self.written);
        self.digits_literal = self.collides && has_digit;
    }
}

/// Whether `written` collides with ordinary speech: the whole form, or the letter run that
/// ends at a letter and digit boundary (`PHD2`), or the capitals that end at a case
/// boundary (`CRISPRi`), is an ordinary English word or a listed abbreviation. A word the
/// list holds only capitalized (a brand such as `Ventolin`) does not collide.
pub fn collides(written: &str) -> bool {
    if words::is_common(written) {
        return true;
    }
    let chars: Vec<char> = written.chars().collect();
    if case_boundary_collides(&chars) {
        return true;
    }
    // The letter run before a digit; a run of one letter is not a word here.
    let run = chars.iter().take_while(|c| c.is_alphabetic()).count();
    if run < 2 || run == chars.len() {
        return false;
    }
    let digit_follows = match chars.get(run) {
        Some(c) if c.is_ascii_digit() => true,
        Some('-') => chars.get(run + 1).is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    };
    let prefix: String = chars.iter().take(run).collect();
    digit_follows && words::is_common(&prefix)
}

/// `CRISPRi`, `CRISPRa`: capitals followed by one or two small letters at the end, whose
/// capitals are a word or abbreviation.
fn case_boundary_collides(chars: &[char]) -> bool {
    let caps = chars.iter().take_while(|c| c.is_uppercase()).count();
    let rest = chars.get(caps..).unwrap_or_default();
    if caps < 2 || rest.is_empty() || rest.len() > 2 || !rest.iter().all(|c| c.is_lowercase()) {
        return false;
    }
    let prefix: String = chars.iter().take(caps).collect();
    words::is_common(&prefix)
}

/// Merges twins: rows with the same written form, one per tier (DD-35). The union of
/// spoken forms, active misrecognitions and cues; the larger usage figures; pinned if
/// either is pinned; the stricter `match` (`prompt_only` over `exact` over `fuzzy`);
/// `bias = never` if either says so. The public row's id is kept.
fn merge_twins(mut entries: Vec<RecognitionEntry>) -> Vec<RecognitionEntry> {
    // Public first, so a merged row carries the public row's id.
    entries.sort_by(|a, b| a.written.cmp(&b.written).then(a.tier.cmp(&b.tier)));
    let mut merged: Vec<RecognitionEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        match merged.last_mut() {
            Some(last) if last.written == entry.written => {
                for (into, from) in [
                    (&mut last.spoken_forms, &entry.spoken_forms),
                    (&mut last.misrecognitions, &entry.misrecognitions),
                    (&mut last.context_cues, &entry.context_cues),
                ] {
                    for form in from {
                        if !into.contains(form) {
                            into.push(form.clone());
                        }
                    }
                }
                last.uses = last.uses.max(entry.uses);
                last.corrected_to = last.corrected_to.max(entry.corrected_to);
                last.last_used_at = last.last_used_at.max(entry.last_used_at);
                last.updated_at = last.updated_at.max(entry.updated_at);
                last.kind = last.kind.or(entry.kind);
                last.match_policy = stricter(last.match_policy, entry.match_policy);
                last.bias = match (last.bias, entry.bias) {
                    (BiasPolicy::Never, _) | (_, BiasPolicy::Never) => BiasPolicy::Never,
                    (BiasPolicy::Always, _) | (_, BiasPolicy::Always) => BiasPolicy::Always,
                    _ => BiasPolicy::Auto,
                };
            }
            _ => merged.push(entry),
        }
    }
    merged
}

fn stricter(a: MatchPolicy, b: MatchPolicy) -> MatchPolicy {
    let rank = |m: MatchPolicy| match m {
        MatchPolicy::Fuzzy => 0,
        MatchPolicy::Exact => 1,
        MatchPolicy::PromptOnly => 2,
    };
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// The entries one dictation may use, and which decoder bonus table goes with them.
///
/// One value feeds the token prompt, the decoder bonus, the alias automaton, the exact key
/// matcher and the fuzzy matcher (DD-36), so no channel can use a row another would not.
#[derive(Clone, Debug)]
pub struct EligibleEntries {
    entries: Arc<[RecognitionEntry]>,
    table: BonusTable,
    scope: BiasScope,
    revision: u64,
}

impl EligibleEntries {
    /// Called only from `crates/biorouter/src/dictation/bias.rs`, after the tier and
    /// institution rules; `eligible_entries_are_built_only_by_bias_set` pins every call.
    /// Merges twins and derives each entry's flags. `generation` is the store's in process
    /// generation: the bonus table's revision changes with it, and with the rows, so the
    /// worker rebuilds every table at every dictionary change (oracle rule 9).
    #[doc(hidden)]
    pub fn new_filtered(
        entries: Vec<RecognitionEntry>,
        table: BonusTable,
        scope: BiasScope,
        generation: u64,
    ) -> EligibleEntries {
        let mut entries = merge_twins(entries);
        for entry in &mut entries {
            entry.derive_flags();
        }
        let revision = revision_of(generation, &table, &entries);
        EligibleEntries {
            entries: entries.into(),
            table,
            scope,
            revision,
        }
    }

    /// No entries: `--no-dictionary`, a store that cannot be read, a build without one.
    pub fn empty() -> EligibleEntries {
        EligibleEntries {
            entries: Arc::from(Vec::new()),
            table: BonusTable::PublicRows,
            scope: BiasScope::Public,
            revision: 0,
        }
    }

    pub fn entries(&self) -> &[RecognitionEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn table(&self) -> &BonusTable {
        &self.table
    }

    pub fn scope(&self) -> BiasScope {
        self.scope
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The worker's `set_bias` for this set: every entry the bonus may start, unranked, or
    /// `None` when there is none (DD-46).
    pub fn bias_table(&self, params: BonusParams) -> Option<SetBias> {
        let entries: Vec<BiasEntry> = self
            .entries
            .iter()
            .filter(|e| e.boostable())
            .map(|e| BiasEntry {
                written: e.written.clone(),
                bias: if e.pinned() { "always" } else { "auto" }.to_string(),
            })
            .collect();
        (!entries.is_empty()).then(|| SetBias {
            revision: self.revision,
            scope: self.table.scope(),
            entries,
            params,
        })
    }
}

/// FNV-1a over the generation, the table and each entry's written form and bonus policy:
/// the bonus table's revision. It changes at every store generation and whenever the rows
/// the table holds change (a mixing policy that changed while the store did not).
fn revision_of(generation: u64, table: &BonusTable, entries: &[RecognitionEntry]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    eat(&generation.to_le_bytes());
    eat(table.scope().as_bytes());
    for entry in entries {
        eat(entry.written.as_bytes());
        eat(&[u8::from(entry.boostable()), u8::from(entry.pinned())]);
    }
    // Zero is the empty set's revision.
    hash.max(1)
}

#[cfg(test)]
pub(crate) mod test_entries {
    //! Entries for the recognition tests, built without `EligibleEntries`.

    use super::*;

    pub(crate) fn entry(written: &str) -> RecognitionEntry {
        let mut e = RecognitionEntry {
            id: format!("dct_{}", written.to_lowercase()),
            tier: EntryTier::Public,
            written: written.to_string(),
            spoken_forms: Vec::new(),
            misrecognitions: Vec::new(),
            context_cues: Vec::new(),
            kind: None,
            match_policy: MatchPolicy::default_for(None, written),
            bias: BiasPolicy::Auto,
            uses: 0,
            last_used_at: None,
            corrected_to: 0,
            updated_at: DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default(),
            collides: false,
            digits_literal: false,
            letter_digit: false,
        };
        e.derive_flags();
        e
    }

    pub(crate) fn with(
        mut e: RecognitionEntry,
        f: impl FnOnce(&mut RecognitionEntry),
    ) -> RecognitionEntry {
        f(&mut e);
        e.derive_flags();
        e
    }
}

#[cfg(test)]
mod tests {
    use super::test_entries::{entry, with};
    use super::*;

    #[test]
    fn the_derived_flags() {
        let flags = |w: &str| {
            let e = entry(w);
            (e.collides, e.digits_literal, e.letter_digit)
        };
        assert_eq!(flags("SPOKE"), (true, false, false));
        assert_eq!(flags("PHD2"), (true, true, true));
        assert_eq!(flags("CRISPRi"), (true, false, false));
        assert_eq!(flags("EPAS1"), (false, false, true));
        assert_eq!(flags("Biorouter"), (false, false, false));
        assert_eq!(
            flags("Ventolin"),
            (false, false, false),
            "a brand does not collide"
        );
        assert_eq!(flags("CD4"), (true, true, true));
        assert_eq!(flags("HIF-2α"), (false, false, true));
        assert!(!entry("UCSF").collides);
    }

    /// Twins merge in `EligibleEntries` (PR-14's acceptance of DD-35): the union of forms,
    /// the larger usage, the stricter match, never over pinned, the public id.
    #[test]
    fn twins_merge() {
        let public = with(entry("EPAS1"), |e| {
            e.spoken_forms = vec!["epas one".into()];
            e.uses = 3;
            e.match_policy = MatchPolicy::Fuzzy;
        });
        let private = with(entry("EPAS1"), |e| {
            e.id = "dct_private".into();
            e.tier = EntryTier::Private;
            e.spoken_forms = vec!["e pass one".into(), "epas one".into()];
            e.misrecognitions = vec!["a pass one".into()];
            e.uses = 7;
            e.corrected_to = 2;
            e.match_policy = MatchPolicy::Exact;
            e.bias = BiasPolicy::Always;
        });
        let other = entry("UCSF");
        let merged = merge_twins(vec![private, other, public]);
        assert_eq!(merged.len(), 2);
        let epas = merged.iter().find(|e| e.written == "EPAS1").unwrap();
        assert_eq!(epas.id, "dct_epas1", "the public row's id");
        assert_eq!(epas.spoken_forms, ["epas one", "e pass one"]);
        assert_eq!(epas.misrecognitions, ["a pass one"]);
        assert_eq!((epas.uses, epas.corrected_to), (7, 2));
        assert_eq!(epas.match_policy, MatchPolicy::Exact);
        assert_eq!(epas.bias, BiasPolicy::Always);

        let never = with(entry("EPAS1"), |e| e.bias = BiasPolicy::Never);
        let pinned = with(entry("EPAS1"), |e| {
            e.tier = EntryTier::Private;
            e.bias = BiasPolicy::Always;
        });
        assert_eq!(merge_twins(vec![never, pinned])[0].bias, BiasPolicy::Never);
    }

    #[test]
    fn the_bonus_table_holds_what_the_bonus_may_start() {
        let set = EligibleEntries {
            entries: vec![
                entry("EPAS1"),
                entry("SPOKE"),
                with(entry("PHD2"), |e| e.bias = BiasPolicy::Always),
                with(entry("VHL"), |e| e.bias = BiasPolicy::Never),
            ]
            .into(),
            table: BonusTable::PrivateRows("ucsf".into()),
            scope: BiasScope::Private,
            revision: 9,
        };
        let table = set.bias_table(BonusParams::default()).expect("a table");
        assert_eq!(table.scope, "private:ucsf");
        assert_eq!(table.revision, 9);
        let written: Vec<(&str, &str)> = table
            .entries
            .iter()
            .map(|e| (e.written.as_str(), e.bias.as_str()))
            .collect();
        assert_eq!(written, [("EPAS1", "auto"), ("PHD2", "always")]);
        assert!(EligibleEntries::empty()
            .bias_table(BonusParams::default())
            .is_none());
    }

    #[test]
    fn the_revision_follows_the_generation_and_the_rows() {
        let rows = vec![entry("EPAS1")];
        let a = revision_of(1, &BonusTable::PublicRows, &rows);
        assert_eq!(a, revision_of(1, &BonusTable::PublicRows, &rows));
        assert_ne!(a, revision_of(2, &BonusTable::PublicRows, &rows));
        assert_ne!(a, revision_of(1, &BonusTable::AllRows, &rows));
        assert_ne!(
            a,
            revision_of(1, &BonusTable::PublicRows, &[entry("EPAS2")])
        );
    }
}
