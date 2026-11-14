//! The one bias set (`docs/dictation/dictionary.md`, "The bias set", DD-36).
//!
//! [`bias_set`] is the only producer of the entries recognition may use. One value feeds
//! the token prompt, the decoder bonus, the alias automaton, the exact key matcher and the
//! fuzzy matcher, so no channel can use a row another would not. The leaf crate defines
//! the newtype ([`EligibleEntries`]); `eligible_entries_are_built_only_by_bias_set` pins
//! every call of its constructor to this file.
//!
//! The predicate is the plan's:
//!
//! ```text
//! status = active  AND  (not enforced  OR  tier = public  OR  (target private AND standing private))
//! ```
//!
//! compared through [`privacy::visible_to`], and a private row then passes only when every
//! institution it names admits the target's model, each asked through
//! [`affiliation::gate_cross_affiliation_warning`] with the row standing in for a private
//! extension whose allowlist is that one institution (O-32, O-35, P-16). `null` asks
//! nothing; `[]` asks once with an empty allowlist, which only a local model clears. The
//! master switch off and the mixing policy `open` lift that check as they lift it for
//! extensions; when the person includes private terms for one recording (O-8), every
//! private row passes, institution tagged rows included (O-37).
//!
//! `bias = never` rows stay in the set: they never reach the prompt or the decoder bonus,
//! and correction still uses them ("Field effects per channel"). DD-36's predicate also
//! names `bias ≠ never`; the field table is the more specific statement and the one this
//! file follows.
//!
//! [`DictionaryRequest`] is what one dictation then carries: the ranked plan for its
//! context, the corrector built for it, and the decoder bonus table, all built when the
//! dictation starts so the path from stop to text pays for none of them. A daemon builds
//! its requests through one [`DictionaryCache`], which keeps what does not depend on the
//! request from one dictation to the next while the store is unchanged (PERF-1). After the
//! text is out, [`DictionaryRequest::usage_bumps`] says which rows it used and
//! [`record_usage`] writes them to `usage.json` ("Usage").

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use biorouter_dictation::protocol::{BonusParams, SetBias};
use biorouter_dictation::recognize::correct::{Change, Corrected, CorrectionIndex, Corrector};
use biorouter_dictation::recognize::prompt;
use biorouter_dictation::recognize::select::{self, RankIndex, RequestPlan};
use biorouter_dictation::recognize::{
    words, BiasScope, BonusTable, EligibleEntries, RecognitionEntry,
};
use biorouter_dictation::store::{
    DictionaryStore, Entry, EntryId, EntryTier, StoreView, UsageDelta, UsageScope,
};
use chrono::{DateTime, Utc};

use crate::managed::{ManagedPolicy, Rule};
use crate::privacy::{
    self, affiliation, ExtensionClassification, ModelAffiliation, ProviderTier,
    SessionClassification,
};

/// Whether private entries may shape this caller's text: the user action proof, a private
/// capability stated in `X-Caller-Provider`, or a person at an interactive terminal. The
/// field is private, so no crate can write `Standing { private: true }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Standing {
    private: bool,
}

impl Standing {
    pub const fn public() -> Standing {
        Standing { private: false }
    }

    /// Built only where a caller's standing was checked: the terminal rule of DD-77
    /// (`target::cli_bias`) and, from PR-15, `HttpCaller::dictation_standing`.
    #[doc(hidden)]
    pub const fn from_checked(private: bool) -> Standing {
        Standing { private }
    }

    pub const fn is_private(&self) -> bool {
        self.private
    }
}

/// The model a dictation is aimed at. `private` comes from `chat_reach`, the new chat bind
/// or the CLI rule of DD-77; `affiliation` from the same provider read (`None`: unstated
/// or unknown); `person_included` is `include_private_terms` or `--private-terms`, admitted
/// under O-8, and covers institution tagged rows too (O-37).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BiasTarget {
    pub private: bool,
    pub affiliation: Option<ModelAffiliation>,
    pub person_included: bool,
}

impl BiasTarget {
    /// A public target: clipboard, Crew, an app, an unknown CLI target, `transcribe`
    /// without an admitted `--session`.
    pub fn public() -> BiasTarget {
        BiasTarget::default()
    }
}

/// Which channels shape a dictation (DD-44). M-22 decided on 2026-11-06 that the declared
/// fallback ships: the token prompt alone, with the decoder bonus built but off, because
/// the two together raised insertions on the probe clips and reversed speech
/// (`speech-model-benchmarks.md`, "The M-22 channel decision"). Correction always runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiasChannels {
    pub prompt: bool,
    pub bonus: bool,
}

/// Turns the decoder bonus on beside the prompt when it is exactly `1`, and read only by a
/// debug build, like `BIOROUTER_DICTATION_TEST_MODEL`: a switch for measuring the bonus
/// through `biorouter transcribe`, read only by [`BiasChannels::for_dictation`]. A release
/// build ignores it, so what ships decodes with [`BiasChannels::DEFAULT`] whatever its
/// environment holds; the bonus's release cost is measured by the booster's callback
/// benchmark instead. It is not a setting: no interface, configuration key or route sets
/// it, and it changes no privacy rule, because the bonus reads the same bias set as the
/// prompt.
pub const MEASURE_BONUS_ENV: &str = "BIOROUTER_DICTATION_MEASURE_BONUS";

impl BiasChannels {
    /// The compiled default: the token prompt only (M-22's outcome).
    pub const DEFAULT: BiasChannels = BiasChannels {
        prompt: true,
        bonus: false,
    };
    /// Both channels, M-22's "both" arm: for measurement only.
    pub const BOTH: BiasChannels = BiasChannels {
        prompt: true,
        bonus: true,
    };
    /// Correction only, for the accuracy test's arms.
    pub const NONE: BiasChannels = BiasChannels {
        prompt: false,
        bonus: false,
    };

    /// The channels a dictation uses: [`Self::DEFAULT`], or in a debug build [`Self::BOTH`]
    /// when [`MEASURE_BONUS_ENV`] is `1`.
    pub fn for_dictation() -> BiasChannels {
        if !cfg!(debug_assertions) {
            return Self::DEFAULT;
        }
        Self::from_measure_switch(std::env::var_os(MEASURE_BONUS_ENV).as_deref())
    }

    fn from_measure_switch(value: Option<&std::ffi::OsStr>) -> BiasChannels {
        if value.is_some_and(|v| v == "1") {
            Self::BOTH
        } else {
            Self::DEFAULT
        }
    }
}

impl Default for BiasChannels {
    fn default() -> BiasChannels {
        BiasChannels::DEFAULT
    }
}

fn classification_of(tier: EntryTier) -> SessionClassification {
    match tier {
        EntryTier::Public => SessionClassification::Public,
        EntryTier::Private => SessionClassification::Private,
    }
}

/// The key of a private rows table for `affiliation` (oracle rule 9: one table per target
/// affiliation in use).
fn affiliation_key(affiliation: Option<&ModelAffiliation>) -> String {
    match affiliation {
        None => "unstated".to_string(),
        Some(ModelAffiliation::Local) => "local".to_string(),
        Some(ModelAffiliation::Institutions(set)) => {
            let ids: Vec<&str> = set.iter().map(|id| id.as_str()).collect();
            ids.join("+")
        }
    }
}

/// The entries one dictation aimed at `target`, for a caller with `standing`, may use.
/// `enforced` is the privacy master switch (DR-15). Under the managed `dictionary: false`
/// key it is [`EligibleEntries::empty`] whatever the store holds ("Managed policy keys"):
/// no prompt, no bonus, no correction and so no usage. The policy is read here, at every
/// call, as every managed key is read when it is asked, so no caller can forget it; the
/// read sits where the dictation is prepared, off the path from stop to text.
pub fn bias_set(
    view: &StoreView,
    target: &BiasTarget,
    standing: Standing,
    enforced: bool,
) -> EligibleEntries {
    let policy = ManagedPolicy::load();
    bias_set_under(
        &policy.dictation().dictionary,
        view,
        target,
        standing,
        enforced,
    )
}

/// [`bias_set`] under the managed `dictionary` rule `dictionary`.
fn bias_set_under(
    dictionary: &Rule,
    view: &StoreView,
    target: &BiasTarget,
    standing: Standing,
    enforced: bool,
) -> EligibleEntries {
    match decide(dictionary, view, target, standing, enforced) {
        Some(decided) => copy_rows(view, &decided),
        None => EligibleEntries::empty(),
    }
}

/// What the bias set's rules decided for one dictation, before any row is copied: its
/// table, its scope, and which rows of the view passed, by their place in it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Decided {
    table: BonusTable,
    scope: BiasScope,
    rows: Vec<u32>,
}

/// The predicate, for every row: `None` under the managed `dictionary: false` key. Every
/// dictation runs it in full, a daemon's included ([`DictionaryCache`]), so no rule a set
/// was once admitted under can stand in for the rules as they are now.
fn decide(
    dictionary: &Rule,
    view: &StoreView,
    target: &BiasTarget,
    standing: Standing,
    enforced: bool,
) -> Option<Decided> {
    if !dictionary.is_allowed() {
        return None;
    }
    // Private reach only for a private capable target AND a caller with private standing.
    let reach =
        if !enforced || ((target.private || target.person_included) && standing.is_private()) {
            ProviderTier::Private
        } else {
            ProviderTier::Public
        };
    let (table, scope) = match reach {
        ProviderTier::Public => (BonusTable::PublicRows, BiasScope::Public),
        ProviderTier::Private if !enforced => (BonusTable::AllRows, BiasScope::Public),
        ProviderTier::Private if target.person_included => {
            (BonusTable::AllRows, BiasScope::Private)
        }
        ProviderTier::Private => (
            BonusTable::PrivateRows(affiliation_key(target.affiliation.as_ref())),
            BiasScope::Private,
        ),
    };
    let rows = view
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| RecognitionEntry::is_recognizable(e))
        .filter(|(_, e)| privacy::visible_to(reach, classification_of(e.tier)))
        .filter(|(_, e)| target.person_included || admits(e, target, reach, enforced))
        .map(|(at, _)| at as u32)
        .collect();
    Some(Decided { table, scope, rows })
}

/// The rows `decided` admitted, as recognition reads them, with the counters of the map
/// their table ranks by.
fn copy_rows(view: &StoreView, decided: &Decided) -> EligibleEntries {
    let usage = decided.table.usage_scope();
    let entries = view.entries();
    let rows: Vec<RecognitionEntry> = decided
        .rows
        .iter()
        .filter_map(|&at| entries.get(at as usize))
        .map(|e| RecognitionEntry::from_entry(e, view.usage(&e.id, usage)))
        .collect();
    // The bonus table's revision follows the entries file's content stamp, never a count
    // of reads: an unchanged store and target give the worker the revision it already
    // holds, so a dictation sends no table again, while every write, in either tier, gives
    // every table a new revision (oracle rule 9). Usage moves no revision: the table holds
    // written forms and bias policies, never counters.
    EligibleEntries::new_filtered(
        rows,
        decided.table.clone(),
        decided.scope,
        view.content_stamp(),
    )
}

/// How many sets a [`DictionaryCache`] keeps: one per table a person dictates into (the
/// public rows, a private table per model affiliation, every row), most recent first.
const CACHED_SETS: usize = 4;

/// What a daemon's dictations build from an unchanged dictionary, kept across them
/// (PERF-1). Without it every stream built its set, its ranking and its corrector from all
/// 5,000 rows; in a release build the ranking alone took 3.0 to 3.2 ms and the corrector 1.8
/// to 2.0 ms with bare rows, 16.9 to 19.0 ms with a spoken form and a misrecognition each
/// [M, 2026-11-07]. The cache keeps, per set, the rows as recognition reads them and the
/// parts of the ranking and of the corrector that no request changes ([`RankIndex`],
/// [`CorrectionIndex`]); each request then ranks for its own context and time and builds
/// the few fuzzy keys its plan names.
///
/// A set is found by what the rules decided, never by who asked: the entries file's
/// content stamp, the table, the scope and the very rows that passed. The predicate runs in
/// full for every dictation ([`decide`]), so a change of target, standing, master switch,
/// mixing policy or managed key that admits other rows finds another set, or builds one.
/// A write in either tier gives the entries file a new content stamp, and every set of the
/// old one is dropped. A usage flush moves only the counters: the rows are copied again,
/// with their new figures, and the indexes are kept once the copy is checked to hold the
/// same rows. After a write of its own, and after each usage flush, the daemon builds the
/// recent sets again from the new files ([`DictionaryCache::prewarm`]), so the dictation
/// after it finds its set ready.
///
/// A set is built once at a time: a dictation that needs a set another thread is building
/// (the rebuild after a write, a stream that started first) waits for that build and takes
/// it, rather than building the same set beside it. The PERF-1 review measured the
/// duplicate in a release build at 6.7 ms with bare rows and 22 to 23 ms with forms, where
/// the set once built costs 0.3 ms [M, 2026-11-07].
#[derive(Default)]
pub struct DictionaryCache {
    sets: Mutex<Sets>,
    /// Signalled whenever a build ends, for the dictations that wait for it.
    built: Condvar,
    /// The last few dictations' inputs, distinct, most recent first, so a write the daemon
    /// makes can build their sets again before the next dictation asks ([`Self::prewarm`]).
    recent: Mutex<Vec<Recent>>,
    copies: AtomicU64,
    indexes: AtomicU64,
    prewarms: AtomicU64,
    waits: AtomicU64,
}

/// What a [`DictionaryCache`] holds under its lock.
#[derive(Default)]
struct Sets {
    /// The sets kept, most recent first.
    kept: Vec<CachedSet>,
    /// The sets being built at this moment, by the entries file they are built from and the
    /// rows they hold.
    building: Vec<(u64, Decided)>,
}

/// Marks one set as being built for as long as it lives. Dropping it, when the build ends
/// or if it panics, takes the mark away and wakes every dictation waiting for the set, so
/// none waits for a build that will never end.
struct Flight<'a> {
    cache: &'a DictionaryCache,
    content: u64,
    decided: Decided,
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        {
            let mut sets = self.cache.sets();
            if let Some(at) = sets
                .building
                .iter()
                .position(|(content, decided)| *content == self.content && *decided == self.decided)
            {
                sets.building.swap_remove(at);
            }
        }
        self.cache.built.notify_all();
    }
}

/// What one dictation asked the cache for.
#[derive(Clone, Debug, PartialEq)]
struct Recent {
    target: BiasTarget,
    standing: Standing,
    enforced: bool,
}

struct CachedSet {
    content: u64,
    usage: u64,
    decided: Decided,
    eligible: EligibleEntries,
    rank: Arc<RankIndex>,
    correction: Arc<CorrectionIndex>,
}

/// A set as a request takes it: the rows and the two indexes.
type SetParts = (EligibleEntries, Arc<RankIndex>, Arc<CorrectionIndex>);

impl CachedSet {
    fn parts(&self) -> SetParts {
        (
            self.eligible.clone(),
            Arc::clone(&self.rank),
            Arc::clone(&self.correction),
        )
    }
}

impl DictionaryCache {
    pub const fn new() -> DictionaryCache {
        DictionaryCache {
            sets: Mutex::new(Sets {
                kept: Vec::new(),
                building: Vec::new(),
            }),
            built: Condvar::new(),
            recent: Mutex::new(Vec::new()),
            copies: AtomicU64::new(0),
            indexes: AtomicU64::new(0),
            prewarms: AtomicU64::new(0),
            waits: AtomicU64::new(0),
        }
    }

    /// The [`DictionaryRequest`] a dictation aimed at `target`, for a caller with
    /// `standing`, gets from `view`: what `DictionaryRequest::new` over [`bias_set`] gives,
    /// reusing what an earlier dictation over the same rows built. It runs the bias set's
    /// predicate in full, so it is a privacy guard of its own: the wiring census
    /// (`crates/biorouter/tests/privacy_guard_wiring.rs`) holds its callers, the daemon's
    /// dictation stream among them.
    pub fn bias_request(
        &self,
        view: &StoreView,
        target: &BiasTarget,
        standing: Standing,
        enforced: bool,
        context: &str,
        channels: BiasChannels,
    ) -> DictionaryRequest {
        {
            let asked = Recent {
                target: target.clone(),
                standing,
                enforced,
            };
            let mut recent = lock(&self.recent);
            recent.retain(|other| *other != asked);
            recent.insert(0, asked);
            recent.truncate(CACHED_SETS);
        }
        let policy = ManagedPolicy::load();
        self.bias_request_under(
            &policy.dictation().dictionary,
            view,
            target,
            standing,
            enforced,
            context,
            channels,
        )
    }

    /// Builds again, from `view`, the sets the last few dictations asked for, so the
    /// dictation after a write finds its set ready: the daemon calls it after each of its own
    /// writes, off the request and after the answer, and after each usage flush, which moves
    /// only the counters, so the rows are copied again and the indexes kept, after the text
    /// is out rather than in the next stream's creation. A dictation that asks for one of
    /// these sets while it is being built waits for it. Each decision runs here in full, with
    /// the rules as they are now, and a set built here serves a dictation only when that
    /// dictation's own decision admits exactly its rows, so an input remembered here grants
    /// nothing. A view of no file builds nothing.
    pub fn prewarm(&self, view: &StoreView) {
        if view.content_stamp() != 0 {
            let policy = ManagedPolicy::load();
            let recent = lock(&self.recent).clone();
            for asked in recent {
                let decided = decide(
                    &policy.dictation().dictionary,
                    view,
                    &asked.target,
                    asked.standing,
                    asked.enforced,
                );
                if let Some(decided) = decided.filter(|d| !d.rows.is_empty()) {
                    self.set_for(view, decided);
                }
            }
        }
        self.prewarms.fetch_add(1, Ordering::Relaxed);
    }

    /// [`Self::bias_request`] under the managed `dictionary` rule `dictionary`.
    #[allow(clippy::too_many_arguments)]
    fn bias_request_under(
        &self,
        dictionary: &Rule,
        view: &StoreView,
        target: &BiasTarget,
        standing: Standing,
        enforced: bool,
        context: &str,
        channels: BiasChannels,
    ) -> DictionaryRequest {
        let Some(decided) = decide(dictionary, view, target, standing, enforced) else {
            return DictionaryRequest::new(EligibleEntries::empty(), context, channels);
        };
        // An empty set, or a view of no file, costs nothing to build and is not kept.
        if decided.rows.is_empty() || view.content_stamp() == 0 {
            return DictionaryRequest::new(copy_rows(view, &decided), context, channels);
        }
        let (eligible, rank, correction) = self.set_for(view, decided);
        DictionaryRequest::indexed(eligible, &rank, correction, context, channels)
    }

    fn sets(&self) -> std::sync::MutexGuard<'_, Sets> {
        lock(&self.sets)
    }

    /// The set of `decided` rows of `view`, with its indexes: kept, or built and kept. While
    /// another thread builds this very set from the same entries file, this one waits for
    /// it and looks again, so a set is never built twice at once.
    fn set_for(&self, view: &StoreView, decided: Decided) -> SetParts {
        let (content, usage) = (view.content_stamp(), view.usage_stamp());
        let mut sets = self.sets();
        let earlier = loop {
            // A set of another version of the entries file can never serve again.
            sets.kept.retain(|set| set.content == content);
            let at = sets.kept.iter().position(|set| set.decided == decided);
            if let Some(at) = at {
                let set = sets.kept.remove(at);
                sets.kept.insert(0, set);
                if sets.kept[0].usage == usage {
                    return sets.kept[0].parts();
                }
            }
            let in_flight = sets
                .building
                .iter()
                .any(|(other, rows)| *other == content && *rows == decided);
            if !in_flight {
                break at.map(|_| sets.kept[0].parts());
            }
            self.waits.fetch_add(1, Ordering::Relaxed);
            sets = self
                .built
                .wait(sets)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        };
        sets.building.push((content, decided.clone()));
        drop(sets);
        let flight = Flight {
            cache: self,
            content,
            decided,
        };
        let decided = &flight.decided;
        // Built outside the lock: another table's dictation need not wait for this one.
        let eligible = copy_rows(view, decided);
        self.copies.fetch_add(1, Ordering::Relaxed);
        let (rank, correction) = match earlier {
            Some((old, rank, correction)) if same_rows(old.entries(), eligible.entries()) => {
                (rank, correction)
            }
            _ => {
                words::warm();
                self.indexes.fetch_add(1, Ordering::Relaxed);
                (
                    Arc::new(RankIndex::new(&eligible)),
                    Arc::new(CorrectionIndex::new(&eligible)),
                )
            }
        };
        {
            let mut sets = self.sets();
            sets.kept
                .retain(|set| set.content == content && set.decided != *decided);
            sets.kept.insert(
                0,
                CachedSet {
                    content,
                    usage,
                    decided: decided.clone(),
                    eligible: eligible.clone(),
                    rank: Arc::clone(&rank),
                    correction: Arc::clone(&correction),
                },
            );
            sets.kept.truncate(CACHED_SETS);
        }
        // The set is kept before the mark goes, so a dictation woken by it finds the set.
        drop(flight);
        (eligible, rank, correction)
    }

    /// How many times this cache copied rows out of a view into a set: once per set and
    /// version of the counters. The latency tests read it.
    pub fn rows_copied(&self) -> u64 {
        self.copies.load(Ordering::Relaxed)
    }

    /// How many times it built a set's ranking and correction indexes: once per set and
    /// version of the entries file.
    pub fn indexes_built(&self) -> u64 {
        self.indexes.load(Ordering::Relaxed)
    }

    /// How many [`Self::prewarm`] runs have finished, for a test that waits for one.
    pub fn prewarms(&self) -> u64 {
        self.prewarms.load(Ordering::Relaxed)
    }

    /// How many times a dictation, or a rebuild, waited for a set another thread was
    /// building instead of building it too. The latency tests print it.
    pub fn waits(&self) -> u64 {
        self.waits.load(Ordering::Relaxed)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether two copies hold the same rows in the same order, whatever their usage counters:
/// the only fields neither index reads.
fn same_rows(a: &[RecognitionEntry], b: &[RecognitionEntry]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_row(a, b))
}

/// [`same_rows`] for one row. Every field is named, so a field added to
/// [`RecognitionEntry`] is a compile error here until someone decides which side it is on.
fn same_row(a: &RecognitionEntry, b: &RecognitionEntry) -> bool {
    let RecognitionEntry {
        id,
        tier,
        written,
        spoken_forms,
        misrecognitions,
        context_cues,
        kind,
        match_policy,
        bias,
        uses: _,
        last_used_at: _,
        corrected_to: _,
        updated_at,
        collides,
        digits_literal,
        letter_digit,
    } = a;
    *id == b.id
        && *tier == b.tier
        && *written == b.written
        && *spoken_forms == b.spoken_forms
        && *misrecognitions == b.misrecognitions
        && *context_cues == b.context_cues
        && *kind == b.kind
        && *match_policy == b.match_policy
        && *bias == b.bias
        && *updated_at == b.updated_at
        && *collides == b.collides
        && *digits_literal == b.digits_literal
        && *letter_digit == b.letter_digit
}

/// DR-26 for one row (O-32, O-35, P-16): every institution the row names must admit the
/// target's model, each asked as a private extension whose allowlist is that one
/// institution. A public row names none and so is never refused.
fn admits(entry: &Entry, target: &BiasTarget, reach: ProviderTier, enforced: bool) -> bool {
    let stored = entry
        .affiliation
        .as_ref()
        .map(|row| row.institutions.as_slice());
    affiliation::dictionary_row_allowlists(stored)
        .into_iter()
        .all(|allowlist| {
            let class = ExtensionClassification {
                tier: ProviderTier::Private,
                affiliation: allowlist,
            };
            affiliation::gate_cross_affiliation_warning(
                enforced,
                reach,
                target.affiliation,
                "dictionary",
                &class,
            )
            .is_none()
        })
}

/// What one dictation carries from its dictionary: the eligible entries, the plan for its
/// context (ranking, the prompt's terms, the fuzzy set), the corrector, and the decoder
/// bonus table. Built when the dictation starts.
pub struct DictionaryRequest {
    pub eligible: EligibleEntries,
    pub plan: RequestPlan,
    pub channels: BiasChannels,
    /// Whether the prompt ends with a period after its best term (DD-45, PR-14b): on by
    /// default, off only for the measurement that compares the two.
    pub prompt_tail: bool,
    corrector: Corrector,
    table: Option<Arc<SetBias>>,
}

impl DictionaryRequest {
    /// Ranks `eligible` against `context` (at most 4 KB, ranking only, never logged) and
    /// builds everything correction and the worker need. The English word list is decoded
    /// here on first use, off the path from stop to text; an empty set needs no word list,
    /// because correction of an empty set returns the text as it came.
    pub fn new(
        eligible: EligibleEntries,
        context: &str,
        channels: BiasChannels,
    ) -> DictionaryRequest {
        if !eligible.is_empty() {
            words::warm();
        }
        let plan = select::plan(&eligible, context, chrono::Utc::now());
        let corrector = Corrector::new(&eligible, &plan);
        DictionaryRequest::of(eligible, plan, corrector, channels)
    }

    /// [`Self::new`] with the indexes a [`DictionaryCache`] keeps for `eligible`'s rows: the
    /// same request, ranked and corrected the same way, at the cost of the request's own
    /// part alone.
    fn indexed(
        eligible: EligibleEntries,
        rank: &RankIndex,
        correction: Arc<CorrectionIndex>,
        context: &str,
        channels: BiasChannels,
    ) -> DictionaryRequest {
        let plan = select::plan_with(&eligible, rank, context, chrono::Utc::now());
        let corrector = Corrector::with_index(correction, &plan);
        DictionaryRequest::of(eligible, plan, corrector, channels)
    }

    fn of(
        eligible: EligibleEntries,
        plan: RequestPlan,
        corrector: Corrector,
        channels: BiasChannels,
    ) -> DictionaryRequest {
        let table = channels
            .bonus
            .then(|| eligible.bias_table(BonusParams::default()))
            .flatten()
            .map(Arc::new);
        DictionaryRequest {
            eligible,
            plan,
            channels,
            prompt_tail: true,
            corrector,
            table,
        }
    }

    /// No dictionary: a store that could not be read. It decodes no word list.
    /// `--no-dictionary` sends no dictionary at all and builds not even this.
    pub fn none() -> DictionaryRequest {
        DictionaryRequest::new(EligibleEntries::empty(), "", BiasChannels::NONE)
    }

    /// The prompt's written forms, best last, as the worker is sent them: the best term
    /// followed by a period (`prompt::with_tail`) unless [`Self::prompt_tail`] is off;
    /// empty when the prompt channel is off. The period restored Whisper's final sentence
    /// punctuation, which the bare list dropped (PR-14b, `speech-model-benchmarks.md`, "The
    /// prompt tail").
    pub fn prompt_terms(&self) -> Vec<String> {
        if !self.channels.prompt {
            return Vec::new();
        }
        let terms = self.plan.prompt_terms.clone();
        if self.prompt_tail {
            prompt::with_tail(terms)
        } else {
            terms
        }
    }

    /// The decoder bonus table, when the bonus channel is on and some entry may start it.
    pub fn bias_table(&self) -> Option<Arc<SetBias>> {
        self.table.clone()
    }

    /// What the transcript reports as `bias_scope`.
    pub fn scope(&self) -> BiasScope {
        self.eligible.scope()
    }

    /// Which usage map this dictation's bumps reach (oracle rule 11).
    pub fn usage_scope(&self) -> UsageScope {
        self.eligible.table().usage_scope()
    }

    /// `prompt_sent` as correction reads it: the best term without the tail that
    /// [`Self::prompt_terms`] put after it. The fuzzy matcher's prompt piece guard exempts
    /// the term a span would be corrected to, and it compares written forms, so with the
    /// tail the best term (`Modulite.`) no longer exempted itself and a truncated hearing
    /// of it (`Modulit`) stayed uncorrected. Only the period the tail added is removed: a
    /// best term that ends in its own punctuation (`Ph.D.`) was sent as it is and is read
    /// as it is.
    pub fn correction_prompt<'a>(&self, prompt_sent: &'a [String]) -> Cow<'a, [String]> {
        let (Some(sent), Some(best)) = (prompt_sent.last(), self.plan.prompt_terms.last()) else {
            return Cow::Borrowed(prompt_sent);
        };
        if sent != best && sent.strip_suffix(prompt::TAIL) == Some(best.as_str()) {
            let mut untailed = prompt_sent.to_vec();
            if let Some(last) = untailed.last_mut() {
                last.clone_from(best);
            }
            Cow::Owned(untailed)
        } else {
            Cow::Borrowed(prompt_sent)
        }
    }

    /// Corrects `text`; `prompt_sent` is the prompt the worker was sent, tail included,
    /// which correction reads without the tail ([`Self::correction_prompt`]).
    pub fn correct(&self, text: &str, prompt_sent: &[String]) -> Corrected {
        self.corrector
            .correct(text, &self.correction_prompt(prompt_sent))
    }

    /// Corrects one piece of a transcript (a subtitle cue), its changes dropped: the
    /// changes a transcript lists refer to its whole text.
    pub fn correct_piece(&self, text: &str, prompt_sent: &[String]) -> String {
        self.correct(text, prompt_sent).text
    }

    /// The usage bumps of one dictation whose final text is `text` ("Usage"): `uses` plus
    /// one and `last_used_at` set to `at` for every row of this dictation's bias set whose
    /// written form the text holds as a whole word, into the map its table reaches
    /// ([`Self::usage_scope`], oracle rule 11). Twins merged in the set bump the public
    /// row. Called after the text is out, never before.
    pub fn usage_bumps(
        &self,
        text: &str,
        at: DateTime<Utc>,
    ) -> Vec<(EntryId, UsageScope, UsageDelta)> {
        let scope = self.usage_scope();
        self.eligible
            .used_in(text)
            .filter_map(|entry| EntryId::parse(&entry.id))
            .map(|id| {
                (
                    id,
                    scope,
                    UsageDelta {
                        uses: 1,
                        corrected_to: 0,
                        last_used_at: Some(at),
                    },
                )
            })
            .collect()
    }
}

/// Writes one batch of usage bumps to `usage.json` under the store's lock, waiting at most
/// `budget` for it, after the text they count is out: [`USAGE_FLUSH_BUDGET`] for a process
/// that stays up, `Duration::ZERO` (one try) for a one shot `biorouter transcribe`, whose
/// exit would otherwise wait. It never touches `dictionary.json`, so no row's `rev`
/// changes. `false` when nothing was written: an empty batch (which never creates the
/// store's folder), a lock that stayed busy, or a usage file that could not be read or
/// written. A lost bump is acceptable; it costs ranking, never a term.
///
/// [`USAGE_FLUSH_BUDGET`]: biorouter_dictation::store::USAGE_FLUSH_BUDGET
pub fn record_usage(
    store: &DictionaryStore,
    bumps: &[(EntryId, UsageScope, UsageDelta)],
    budget: Duration,
) -> bool {
    if bumps.is_empty() {
        return false;
    }
    store.flush_usage(bumps, budget).unwrap_or(false)
}

/// The changes of a correction, for a transcript.
pub type Changes = Vec<Change>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::affiliation::InstitutionId;
    use biorouter_dictation::store::schema::now;
    use biorouter_dictation::store::{
        AddOutcome, BiasPolicy, DictionaryStore, EntryStatus, NewTerm, Reach, RowAffiliation,
        USAGE_FLUSH_BUDGET,
    };

    struct Fixture {
        _root: tempfile::TempDir,
        store: DictionaryStore,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::tempdir().expect("a folder");
            let store = DictionaryStore::open(root.path().join("dictionary"));
            Fixture { _root: root, store }
        }

        /// Adds `written` in `tier` with `institutions` (`None` is `null`).
        fn add(&self, written: &str, tier: EntryTier, institutions: Option<&[&str]>) {
            let id = match self
                .store
                .write(|file| file.add(NewTerm::person(written, tier), Reach::All, now()))
                .expect("added")
            {
                AddOutcome::Created { id, .. } => id,
                other => panic!("{other:?}"),
            };
            let affiliation = institutions.map(|ids| RowAffiliation {
                institutions: ids.iter().map(|s| s.to_string()).collect(),
            });
            self.store
                .write(|file| {
                    let entry = file
                        .entries
                        .iter_mut()
                        .find(|e| e.id == id)
                        .expect("the row");
                    entry.affiliation = affiliation;
                    Ok(())
                })
                .expect("stamped");
        }

        /// Fills the store to the active cap with public terms `BRL0000X` and up, each with
        /// a spoken form and an active misrecognition when `forms`, in one write.
        fn fill_to_the_cap(&self, forms: bool) {
            use biorouter_dictation::store::ops::MAX_ACTIVE_PER_TIER;
            use biorouter_dictation::store::{EntrySource, MatchPolicy, Misrecognition};
            let at = now();
            self.store
                .write(|file| {
                    for i in 0..MAX_ACTIVE_PER_TIER {
                        let written = format!("BRL{i:04}X");
                        file.entries.push(Entry {
                            id: EntryId::random(),
                            rev: 1,
                            match_policy: MatchPolicy::default_for(None, &written),
                            written,
                            tier: EntryTier::Public,
                            status: EntryStatus::Active,
                            source: EntrySource::Import,
                            reviewed: true,
                            spoken_forms: if forms {
                                vec![format!("barrel {i} spoken")]
                            } else {
                                Vec::new()
                            },
                            misrecognitions: if forms {
                                vec![Misrecognition {
                                    text: format!("barrel {i} heard"),
                                    count: 2,
                                    last_seen: at,
                                }]
                            } else {
                                Vec::new()
                            },
                            kind: None,
                            bias: BiasPolicy::Auto,
                            context_cues: Vec::new(),
                            note: None,
                            reason: None,
                            suggested_in: None,
                            seen: 1,
                            created_at: at,
                            updated_at: at,
                            provenance: None,
                            affiliation: None,
                        });
                    }
                    Ok(())
                })
                .unwrap();
        }

        fn written(&self, target: &BiasTarget, standing: Standing, enforced: bool) -> Vec<String> {
            let view = self.store.view().expect("a view");
            let set = bias_set(&view, target, standing, enforced);
            let mut written: Vec<String> =
                set.entries().iter().map(|e| e.written.clone()).collect();
            written.sort();
            written
        }
    }

    fn private_target(affiliation: Option<ModelAffiliation>) -> BiasTarget {
        BiasTarget {
            private: true,
            affiliation,
            person_included: false,
        }
    }

    fn inst(id: &str) -> InstitutionId {
        InstitutionId::new(id)
    }

    /// The bias table of PR-14's acceptance: target tier, entry tier, standing and the
    /// master switch.
    #[test]
    fn the_bias_table() {
        let f = Fixture::new();
        f.add("PUBLIC1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        let private = Standing::from_checked(true);
        let public = Standing::public();
        let local = private_target(Some(ModelAffiliation::Local));
        let both = vec!["PRIVATE1".to_string(), "PUBLIC1".to_string()];
        let public_only = vec!["PUBLIC1".to_string()];
        for (target, standing, enforced, want, scope) in [
            (&local, private, true, &both, BiasScope::Private),
            (&local, public, true, &public_only, BiasScope::Public),
            (
                &BiasTarget::public(),
                private,
                true,
                &public_only,
                BiasScope::Public,
            ),
            (
                &BiasTarget::public(),
                public,
                true,
                &public_only,
                BiasScope::Public,
            ),
            // The switch off: every row, reported public so the draft flag stays silent.
            (
                &BiasTarget::public(),
                public,
                false,
                &both,
                BiasScope::Public,
            ),
            (&local, public, false, &both, BiasScope::Public),
        ] {
            assert_eq!(
                &f.written(target, standing, enforced),
                want,
                "{target:?} {standing:?} {enforced}"
            );
            let view = f.store.view().unwrap();
            assert_eq!(bias_set(&view, target, standing, enforced).scope(), scope);
        }
        // Clipboard, Crew, an app or an unknown CLI target is public unless the person
        // included private terms for this recording and has private standing (O-8).
        let included = BiasTarget {
            private: false,
            affiliation: None,
            person_included: true,
        };
        assert_eq!(f.written(&included, private, true), both);
        assert_eq!(f.written(&included, public, true), public_only);
    }

    /// Suggested rows never reach recognition; `bias = never` rows stay for correction and
    /// never reach the prompt or the bonus.
    #[test]
    fn active_rows_only_and_never_stays_for_correction() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("EGLN1", EntryTier::Public, None);
        f.add("VHL1", EntryTier::Public, None);
        f.store
            .write(|file| {
                for e in &mut file.entries {
                    match e.written.as_str() {
                        "EGLN1" => e.status = EntryStatus::Suggested,
                        "VHL1" => e.bias = BiasPolicy::Never,
                        _ => {}
                    }
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(
            f.written(&BiasTarget::public(), Standing::public(), true),
            ["EPAS1", "VHL1"]
        );
        let view = f.store.view().unwrap();
        let set = bias_set(&view, &BiasTarget::public(), Standing::public(), true);
        // The bonus is built but off by default (M-22), and measurement turns it on.
        let mut shipped = DictionaryRequest::new(set.clone(), "", BiasChannels::DEFAULT);
        assert_eq!(
            shipped.prompt_terms(),
            ["EPAS1."],
            "the best term, then the tail"
        );
        assert!(shipped.bias_table().is_none(), "no bonus table by default");
        shipped.prompt_tail = false;
        assert_eq!(shipped.prompt_terms(), ["EPAS1"]);
        let request = DictionaryRequest::new(set, "", BiasChannels::BOTH);
        assert_eq!(request.prompt_terms(), ["EPAS1."]);
        let correction_only = DictionaryRequest::new(
            bias_set(&view, &BiasTarget::public(), Standing::public(), true),
            "",
            BiasChannels::NONE,
        );
        assert!(correction_only.prompt_terms().is_empty());
        let table = request.bias_table().expect("a table");
        let boosted: Vec<&str> = table.entries.iter().map(|e| e.written.as_str()).collect();
        assert_eq!(boosted, ["EPAS1"]);
        assert_eq!(table.scope, "public");
        assert_eq!(request.correct("V-H-L-1 here", &[]).text, "VHL1 here");
        assert_eq!(shipped.correct("V-H-L-1 here", &[]).text, "VHL1 here");
    }

    /// DD-44 after M-22 (2026-11-06): the token prompt alone ships, the bonus stays built
    /// and off, and only the measurement switch, set to exactly `1`, turns it on.
    #[test]
    fn the_default_is_the_prompt_alone() {
        assert_eq!(
            BiasChannels::DEFAULT,
            BiasChannels {
                prompt: true,
                bonus: false
            }
        );
        assert_eq!(BiasChannels::default(), BiasChannels::DEFAULT);
        assert_eq!(
            BiasChannels::BOTH,
            BiasChannels {
                prompt: true,
                bonus: true
            }
        );
        use std::ffi::OsStr;
        for (value, want) in [
            (None, BiasChannels::DEFAULT),
            (Some("1"), BiasChannels::BOTH),
            (Some("0"), BiasChannels::DEFAULT),
            (Some("true"), BiasChannels::DEFAULT),
            (Some(""), BiasChannels::DEFAULT),
        ] {
            assert_eq!(
                BiasChannels::from_measure_switch(value.map(OsStr::new)),
                want,
                "{value:?}"
            );
        }
    }

    /// "Usage": a dictation bumps `uses` and `last_used_at` of the rows of its own bias
    /// set that its final text holds as whole words, into the map its table reaches; the
    /// flush leaves `dictionary.json` byte identical and every `rev` as it was.
    #[test]
    fn usage_follows_the_final_text_and_the_table() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("metformin", EntryTier::Public, None);
        f.add("UCSF", EntryTier::Public, None);
        f.add("OCRELIZUMAB", EntryTier::Private, None);
        let view = f.store.view().unwrap();
        let id_of = |written: &str| {
            view.entries()
                .iter()
                .find(|e| e.written == written)
                .unwrap()
                .id
                .clone()
        };
        let at: DateTime<Utc> = "2026-11-06T17:02:11Z".parse().unwrap();
        let text = "Metformin, then EPAS1. EPAS1 again; OCRELIZUMAB, UCSFs and ucsf.";

        // A public dictation: public rows only, in both maps. EPAS1 counts once per
        // dictation; `UCSFs` and `ucsf` are not the written form; a sentence's capital
        // counts for a term written in lower case.
        let public = DictionaryRequest::new(
            bias_set(&view, &BiasTarget::public(), Standing::public(), true),
            "",
            BiasChannels::DEFAULT,
        );
        let bumps = public.usage_bumps(text, at);
        let mut bumped: Vec<(EntryId, UsageScope)> =
            bumps.iter().map(|(id, s, _)| (id.clone(), *s)).collect();
        bumped.sort_by(|a, b| a.0.cmp(&b.0));
        let mut want = vec![
            (id_of("EPAS1"), UsageScope::Public),
            (id_of("metformin"), UsageScope::Public),
        ];
        want.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(bumped, want);
        assert!(bumps
            .iter()
            .all(|(_, _, d)| d.uses == 1 && d.corrected_to == 0 && d.last_used_at == Some(at)));

        // A private dictation: the private row too, in `all` only.
        let private = DictionaryRequest::new(
            bias_set(
                &view,
                &private_target(Some(ModelAffiliation::Local)),
                Standing::from_checked(true),
                true,
            ),
            "",
            BiasChannels::DEFAULT,
        );
        let bumps = private.usage_bumps(text, at);
        assert_eq!(bumps.len(), 3);
        assert!(bumps.iter().all(|(_, s, _)| *s == UsageScope::All));
        assert!(bumps.iter().any(|(id, _, _)| *id == id_of("OCRELIZUMAB")));
        assert!(private.usage_bumps("nothing here", at).is_empty());

        // The flush: counters move, the entries file and its revisions do not.
        let entries_before = std::fs::read(f.store.store_path()).unwrap();
        let revs: Vec<u64> = view.entries().iter().map(|e| e.rev).collect();
        assert!(record_usage(
            &f.store,
            &public.usage_bumps(text, at),
            USAGE_FLUSH_BUDGET
        ));
        assert!(record_usage(
            &f.store,
            &private.usage_bumps(text, at),
            USAGE_FLUSH_BUDGET
        ));
        assert_eq!(std::fs::read(f.store.store_path()).unwrap(), entries_before);
        let after = f.store.view().unwrap();
        assert_eq!(
            after.entries().iter().map(|e| e.rev).collect::<Vec<u64>>(),
            revs
        );
        let epas = after.usage(&id_of("EPAS1"), UsageScope::All).unwrap();
        assert_eq!((epas.uses, epas.last_used_at), (2, Some(at)));
        let epas = after.usage(&id_of("EPAS1"), UsageScope::Public).unwrap();
        assert_eq!(epas.uses, 1, "a private dictation never bumps `public`");
        assert!(after
            .usage(&id_of("OCRELIZUMAB"), UsageScope::Public)
            .is_none());
        // An empty batch writes nothing and creates nothing.
        let empty = tempfile::tempdir().unwrap();
        let untouched = DictionaryStore::open(empty.path().join("dictionary"));
        assert!(!record_usage(&untouched, &[], USAGE_FLUSH_BUDGET));
        assert!(!untouched.dir().exists());
    }

    /// PERF-1: the bonus table's revision holds while the store and the target do, so the
    /// supervisor sends a worker that holds it nothing new at the next dictation: views from
    /// any handle, read any number of times, and a usage flush between them, all give the
    /// revision the first gave. A write in either tier gives a new one (oracle rule 9), and
    /// another target is another table.
    #[test]
    fn the_table_revision_holds_while_the_store_and_the_target_do() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        let public = Standing::public();
        let revision = |store: &DictionaryStore, target: &BiasTarget, standing: Standing| {
            let view = store.view().unwrap();
            let request = DictionaryRequest::new(
                bias_set(&view, target, standing, true),
                "",
                BiasChannels::BOTH,
            );
            let table = request.bias_table().expect("a table");
            (table.scope.clone(), table.revision)
        };
        let first = revision(&f.store, &BiasTarget::public(), public);
        assert_eq!(revision(&f.store, &BiasTarget::public(), public), first);
        let other = DictionaryStore::open(f.store.dir());
        assert_eq!(
            revision(&other, &BiasTarget::public(), public),
            first,
            "a fresh handle reads the same file, so the same revision"
        );
        let id = f.store.view().unwrap().entries()[0].id.clone();
        let bump = UsageDelta {
            uses: 1,
            corrected_to: 0,
            last_used_at: Some(now()),
        };
        assert!(record_usage(
            &other,
            &[(id, UsageScope::Public, bump)],
            USAGE_FLUSH_BUDGET
        ));
        assert_eq!(
            revision(&f.store, &BiasTarget::public(), public),
            first,
            "a usage flush moves no revision"
        );

        let local = private_target(Some(ModelAffiliation::Local));
        let private = revision(&f.store, &local, Standing::from_checked(true));
        assert_ne!(private.0, first.0, "another target is another table");

        // A private write rebuilds the public table too, and its timing says nothing.
        f.add("PRIVATE2", EntryTier::Private, None);
        let after = revision(&f.store, &BiasTarget::public(), public);
        assert_eq!(after.0, first.0);
        assert_ne!(after.1, first.1);
        assert_ne!(
            revision(&f.store, &local, Standing::from_checked(true)).1,
            private.1
        );
    }

    /// Where the dictionary sits on a dictation's path, at the cap (5,000 rows), split into
    /// its parts: the store read (cold parse and warm), the bias set, the ranking and the
    /// corrector a stream builds when it is created, and the correction of one utterance,
    /// which runs between the decode and the text. A timing run, by hand:
    /// `cargo test -p biorouter --lib dictation::bias::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing run, by hand"]
    fn measure_the_dictionary_path_at_the_cap() {
        use biorouter_dictation::recognize::correct::Corrector;
        use std::time::Instant;

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

        // PR-16's live check imported bare terms (`BRL0000X`); the second shape gives every
        // row a spoken form and a misrecognition, so the alias automaton and the key map
        // hold three forms a row.
        for forms in [false, true] {
            let f = Fixture::new();
            f.fill_to_the_cap(forms);
            let bytes = std::fs::metadata(f.store.store_path()).unwrap().len();
            let cold = time(5, || DictionaryStore::open(f.store.dir()).view().unwrap());
            let parses = f.store.store_parses();
            f.store.view().unwrap();
            let warm = time(1_000, || f.store.view().unwrap());
            assert_eq!(
                f.store.store_parses(),
                parses,
                "the write left the cache warm"
            );
            let view = f.store.view().unwrap();
            let target = BiasTarget::public();
            let set = time(9, || bias_set(&view, &target, Standing::public(), true));
            let eligible = bias_set(&view, &target, Standing::public(), true);
            words::warm();
            let plan = time(9, || select::plan(&eligible, "", Utc::now()));
            let ranked = select::plan(&eligible, "", Utc::now());
            let corrector = time(9, || Corrector::new(&eligible, &ranked));
            let request = time(9, || {
                DictionaryRequest::new(eligible.clone(), "", BiasChannels::DEFAULT)
            });
            let dictionary = DictionaryRequest::new(eligible.clone(), "", BiasChannels::DEFAULT);
            let prompt = dictionary.prompt_terms();
            // PERF-1: the daemon's cache. The first request builds the set and its indexes;
            // the rest find them, and after a usage flush only the rows are copied again.
            let cache = DictionaryCache::new();
            let request_of = |view: &StoreView, context: &str| {
                cache.bias_request(
                    view,
                    &target,
                    Standing::public(),
                    true,
                    context,
                    BiasChannels::DEFAULT,
                )
            };
            let first = time(1, || request_of(&view, ""));
            let cached = time(51, || request_of(&view, ""));
            let context = "We talked about the Baranzini group, VHL and barrel 12 spoken.";
            let cached_context = time(51, || request_of(&view, context));
            assert_eq!(cache.indexes_built(), 1, "built once");
            let id = view.entries()[0].id.clone();
            let bump = UsageDelta {
                uses: 1,
                corrected_to: 0,
                last_used_at: Some(now()),
            };
            assert!(record_usage(
                &f.store,
                &[(id, UsageScope::Public, bump)],
                USAGE_FLUSH_BUDGET
            ));
            let flushed = f.store.view().unwrap();
            let after_usage = time(1, || request_of(&flushed, ""));
            assert_eq!(cache.indexes_built(), 1, "a usage flush keeps the indexes");
            // A 5 s utterance, and one that holds terms to correct.
            let plain =
                "The Baranzini group uses spoke to link VHL mutations to disease phenotypes.";
            let terms = "Then barrel 12 heard, BRL0040X and barrel 7 spoken went to the core.";
            let correct_plain = time(201, || dictionary.correct(plain, &prompt));
            let correct_terms = time(201, || dictionary.correct(terms, &prompt));
            println!(
                "{} rows ({}), {bytes} bytes: view cold (parse) {cold:?}; view warm {warm:?}; \
                 bias set {set:?}; plan {plan:?}; corrector {corrector:?}; whole request \
                 {request:?}; the cache: first request {first:?}, then {cached:?} \
                 ({cached_context:?} with context), after a usage flush {after_usage:?}; \
                 correct a plain utterance {correct_plain:?}; correct one with terms \
                 {correct_terms:?} ({:?})",
                eligible.entries().len(),
                if forms {
                    "a spoken form and a misrecognition each"
                } else {
                    "bare"
                },
                dictionary.correct(terms, &prompt).text
            );
        }
    }

    /// What a request carries, for comparing a cached one with one built whole.
    fn assert_same_request(cached: &DictionaryRequest, whole: &DictionaryRequest, what: &str) {
        assert_eq!(
            cached.eligible.entries(),
            whole.eligible.entries(),
            "{what}"
        );
        assert_eq!(cached.eligible.table(), whole.eligible.table(), "{what}");
        assert_eq!(cached.scope(), whole.scope(), "{what}");
        assert_eq!(
            cached.eligible.revision(),
            whole.eligible.revision(),
            "{what}"
        );
        assert_eq!(cached.plan, whole.plan, "{what}");
        assert_eq!(cached.prompt_terms(), whole.prompt_terms(), "{what}");
        assert_eq!(cached.bias_table(), whole.bias_table(), "{what}");
        let prompt = whole.prompt_terms();
        for text in [
            "E-PAS-1 and the bio router, then bee oh router and PRIVATE-1.",
            "U-C-S-F-ROW met EGLN-1 at E PAS one.",
            "Nothing to change here.",
        ] {
            assert_eq!(
                cached.correct(text, &prompt),
                whole.correct(text, &prompt),
                "{what}: {text}"
            );
        }
        let at = now();
        let text = "EPAS1, Biorouter, PRIVATE1, UCSFROW and EGLN1";
        assert_eq!(
            cached.usage_bumps(text, at),
            whole.usage_bumps(text, at),
            "{what}"
        );
    }

    /// PERF-1: a request the daemon's cache gives is the request built whole from
    /// `bias_set`, for every kind of target and both settings of the master switch: from a
    /// set it builds, from one it keeps, from one whose rows it copies again after a usage
    /// flush (keeping its indexes), and after a write in either tier (building new ones).
    #[test]
    fn a_cached_request_is_the_request_built_whole() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        f.add("UCSFROW", EntryTier::Private, Some(&["ucsf"]));
        f.add("Biorouter", EntryTier::Public, None);
        f.store
            .write(|file| {
                for e in &mut file.entries {
                    if e.written == "Biorouter" {
                        e.spoken_forms = vec!["bio router".into(), "bee oh router".into()];
                    }
                }
                Ok(())
            })
            .unwrap();
        let private = Standing::from_checked(true);
        let local = private_target(Some(ModelAffiliation::Local));
        let ucsf = private_target(ModelAffiliation::institutions([inst("ucsf")]));
        let included = BiasTarget {
            person_included: true,
            ..BiasTarget::public()
        };
        // Four tables, as many as the cache keeps: public rows, two private tables, every
        // row; the switch off is the same every row table under another scope, below.
        let cases = [
            (BiasTarget::public(), Standing::public(), true),
            (local.clone(), private, true),
            (ucsf.clone(), private, true),
            (included.clone(), private, true),
        ];
        let cache = DictionaryCache::new();
        let check = |what: &str| {
            let view = f.store.view().unwrap();
            for (target, standing, enforced) in &cases {
                for context in ["", "the bio router and EPAS1"] {
                    let cached = cache.bias_request(
                        &view,
                        target,
                        *standing,
                        *enforced,
                        context,
                        BiasChannels::BOTH,
                    );
                    let whole = DictionaryRequest::new(
                        bias_set(&view, target, *standing, *enforced),
                        context,
                        BiasChannels::BOTH,
                    );
                    assert_same_request(&cached, &whole, &format!("{what}: {target:?}"));
                }
            }
        };
        check("built");
        assert_eq!(cache.indexes_built(), 4, "one set a table");
        assert_eq!(cache.rows_copied(), 4);
        check("kept");
        assert_eq!(
            cache.indexes_built(),
            4,
            "an unchanged store builds nothing"
        );
        assert_eq!(cache.rows_copied(), 4);

        let view = f.store.view().unwrap();
        let id = view.entries()[0].id.clone();
        let bump = UsageDelta {
            uses: 3,
            corrected_to: 1,
            last_used_at: Some(now()),
        };
        assert!(record_usage(
            &f.store,
            &[(id, UsageScope::Public, bump)],
            USAGE_FLUSH_BUDGET
        ));
        check("after a usage flush");
        assert_eq!(cache.rows_copied(), 8, "the counters are copied again");
        assert_eq!(cache.indexes_built(), 4, "and the indexes kept");

        // A private write: every table's set is built again, the public one included.
        f.add("EGLN1", EntryTier::Private, None);
        check("after a private write");
        assert_eq!(cache.indexes_built(), 8);
        f.add("EGLN1", EntryTier::Public, None);
        check("after a public write");
        assert_eq!(cache.indexes_built(), 12);

        // The switch off: every row, scope public, a set of its own.
        let view = f.store.view().unwrap();
        let off = cache.bias_request(
            &view,
            &BiasTarget::public(),
            Standing::public(),
            false,
            "",
            BiasChannels::BOTH,
        );
        let whole = DictionaryRequest::new(
            bias_set(&view, &BiasTarget::public(), Standing::public(), false),
            "",
            BiasChannels::BOTH,
        );
        assert_same_request(&off, &whole, "the switch off");
        assert_eq!(off.scope(), BiasScope::Public);
        assert_eq!(cache.indexes_built(), 13);
        assert!(
            cache.sets().kept.len() <= CACHED_SETS,
            "the cache keeps at most four"
        );

        // Managed `dictionary: false` through the cache: the empty request, nothing kept.
        let forbidden = crate::managed::DictationPolicy::off().dictionary;
        let request = cache.bias_request_under(
            &forbidden,
            &view,
            &local,
            private,
            true,
            "",
            BiasChannels::BOTH,
        );
        assert!(request.eligible.is_empty());
        assert_eq!(request.correct("E-PAS-1 here", &[]).text, "E-PAS-1 here");
        assert_eq!(cache.indexes_built(), 13);
    }

    /// The cache finds a set by the rows the rules admitted, never by the table's name
    /// alone: a mixing policy, a master switch or an affiliation that admits other rows into
    /// a table of the same name gets a set of its own, and no request ever carries a row
    /// its own decision left out (DD-36).
    #[test]
    fn the_cache_never_serves_rows_this_decision_left_out() {
        let f = Fixture::new();
        f.add("PUBLIC1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        f.add("UCSFROW", EntryTier::Private, Some(&["ucsf"]));
        f.add("NRK2", EntryTier::Private, Some(&["other"]));
        let view = f.store.view().unwrap();
        let cache = DictionaryCache::new();
        let all = decide(
            &Rule::Allowed,
            &view,
            &private_target(ModelAffiliation::institutions([inst("ucsf")])),
            Standing::from_checked(true),
            true,
        )
        .unwrap();
        // The same table and scope with one row fewer, as a stricter mixing policy would
        // decide it.
        let mut fewer = all.clone();
        fewer
            .rows
            .retain(|&at| view.entries()[at as usize].written != "UCSFROW");
        assert_eq!(fewer.table, all.table);
        assert_ne!(fewer.rows, all.rows);
        let written = |set: &EligibleEntries| -> Vec<String> {
            let mut written: Vec<String> =
                set.entries().iter().map(|e| e.written.clone()).collect();
            written.sort();
            written
        };
        let (first, _, correction) = cache.set_for(&view, all.clone());
        assert!(written(&first).contains(&"UCSFROW".to_string()));
        let (second, _, second_correction) = cache.set_for(&view, fewer.clone());
        assert!(!written(&second).contains(&"UCSFROW".to_string()));
        assert!(!Arc::ptr_eq(&correction, &second_correction));
        assert_eq!(cache.indexes_built(), 2);
        // Each is kept for its own decision.
        let (again, _, _) = cache.set_for(&view, fewer);
        assert_eq!(written(&again), written(&second));
        let (again, _, _) = cache.set_for(&view, all);
        assert_eq!(written(&again), written(&first));
        assert_eq!(cache.indexes_built(), 2);

        // Through the public entry point: the other institution's row never reaches a
        // UCSF model's request, before or after a request that admitted every row.
        let private = Standing::from_checked(true);
        let included = BiasTarget {
            person_included: true,
            ..BiasTarget::public()
        };
        let every = cache.bias_request(&view, &included, private, true, "", BiasChannels::BOTH);
        assert!(written(&every.eligible).contains(&"NRK2".to_string()));
        assert_eq!(every.correct("N-R-K-2 here", &[]).text, "NRK2 here");
        let ucsf = cache.bias_request(
            &view,
            &private_target(ModelAffiliation::institutions([inst("ucsf")])),
            private,
            true,
            "",
            BiasChannels::BOTH,
        );
        assert_eq!(written(&ucsf.eligible), ["PRIVATE1", "PUBLIC1", "UCSFROW"]);
        assert_eq!(
            ucsf.correct("N-R-K-2 here", &[]).text,
            "N-R-K-2 here",
            "a row of another decision is never corrected to"
        );
    }

    /// After a write, `prewarm` builds again, from the new file, the sets recent dictations
    /// asked for, so the requests after it build nothing; what it built serves a dictation
    /// only when that dictation's own decision admits the same rows.
    #[test]
    fn prewarm_builds_the_recent_sets_from_the_new_file() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        let cache = DictionaryCache::new();
        let local = private_target(Some(ModelAffiliation::Local));
        let private = Standing::from_checked(true);
        let ask = |view: &StoreView, target: &BiasTarget, standing: Standing| {
            cache.bias_request(view, target, standing, true, "", BiasChannels::DEFAULT)
        };
        let view = f.store.view().unwrap();
        ask(&view, &BiasTarget::public(), Standing::public());
        ask(&view, &local, private);
        assert_eq!(cache.indexes_built(), 2);

        f.add("EGLN1", EntryTier::Public, None);
        let view = f.store.view().unwrap();
        cache.prewarm(&view);
        assert_eq!(cache.prewarms(), 1);
        assert_eq!(cache.indexes_built(), 4, "both sets built again");
        let public = ask(&view, &BiasTarget::public(), Standing::public());
        let private_set = ask(&view, &local, private);
        assert_eq!(
            cache.indexes_built(),
            4,
            "the requests after it build nothing"
        );
        assert_eq!(cache.rows_copied(), 4);
        assert_same_request(
            &public,
            &DictionaryRequest::new(
                bias_set(&view, &BiasTarget::public(), Standing::public(), true),
                "",
                BiasChannels::DEFAULT,
            ),
            "the public set prewarm built",
        );
        assert_same_request(
            &private_set,
            &DictionaryRequest::new(
                bias_set(&view, &local, private, true),
                "",
                BiasChannels::DEFAULT,
            ),
            "the private set prewarm built",
        );
        // A caller without private standing aimed at the same model gets the public rows.
        let public_caller = ask(&view, &local, Standing::public());
        assert!(public_caller
            .eligible
            .entries()
            .iter()
            .all(|e| e.written != "PRIVATE1"));
        // A view of no file builds nothing.
        let empty = Fixture::new();
        cache.prewarm(&empty.store.view().unwrap());
        assert_eq!(cache.prewarms(), 2);
        assert_eq!(cache.indexes_built(), 4);
    }

    /// The cache keeps at most four sets, most recent first, and drops every set of an
    /// older entries file at the next request.
    #[test]
    fn the_cache_keeps_four_sets_of_the_current_file() {
        let f = Fixture::new();
        f.add("PUBLIC1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        let private = Standing::from_checked(true);
        let cache = DictionaryCache::new();
        let view = f.store.view().unwrap();
        let ask = |view: &StoreView, institution: &str| {
            cache.bias_request(
                view,
                &private_target(ModelAffiliation::institutions([inst(institution)])),
                private,
                true,
                "",
                BiasChannels::DEFAULT,
            );
        };
        for institution in ["a", "b", "c", "d", "e"] {
            ask(&view, institution);
        }
        assert_eq!(cache.sets().kept.len(), CACHED_SETS);
        assert_eq!(cache.indexes_built(), 5);
        ask(&view, "e");
        assert_eq!(cache.indexes_built(), 5, "the most recent is kept");
        ask(&view, "a");
        assert_eq!(cache.indexes_built(), 6, "the oldest was dropped");
        f.add("PUBLIC2", EntryTier::Public, None);
        let view = f.store.view().unwrap();
        ask(&view, "a");
        assert_eq!(
            cache.sets().kept.len(),
            1,
            "the sets of the older file are gone"
        );
    }

    /// PERF-1 review: a dictation that asks for a set another thread is building waits for
    /// that build and takes it, rather than building the same set beside it, as a stream
    /// that started while the daemon's rebuild after a write ran did. Two requests released
    /// together over 5,000 rows with forms copy and index the rows once between them, and
    /// each is the request built whole; after a write, the rebuild and a request released
    /// together do the same, and no mark is left behind.
    #[test]
    fn a_set_being_built_is_built_once() {
        let f = Fixture::new();
        f.fill_to_the_cap(true);
        let cache = DictionaryCache::new();
        let target = BiasTarget::public();
        let ask = |view: &StoreView| {
            cache.bias_request(
                view,
                &target,
                Standing::public(),
                true,
                "",
                BiasChannels::DEFAULT,
            )
        };
        let barrier = std::sync::Barrier::new(2);
        let view = f.store.view().unwrap();
        let (first, second) = std::thread::scope(|s| {
            let one = s.spawn(|| {
                barrier.wait();
                ask(&view)
            });
            let two = s.spawn(|| {
                barrier.wait();
                ask(&view)
            });
            (one.join().unwrap(), two.join().unwrap())
        });
        assert_eq!(cache.indexes_built(), 1, "the set was indexed once");
        assert_eq!(cache.rows_copied(), 1, "and its rows copied once");
        let whole = DictionaryRequest::new(
            bias_set(&view, &target, Standing::public(), true),
            "",
            BiasChannels::DEFAULT,
        );
        assert_same_request(&first, &whole, "one of the two");
        assert_same_request(&second, &whole, "the other");

        // A write at the cap: the last term goes.
        f.store
            .write(|file| {
                file.entries.pop();
                Ok(())
            })
            .unwrap();
        let view = f.store.view().unwrap();
        std::thread::scope(|s| {
            let rebuild = s.spawn(|| {
                barrier.wait();
                cache.prewarm(&view);
            });
            let stream = s.spawn(|| {
                barrier.wait();
                ask(&view)
            });
            rebuild.join().unwrap();
            stream.join().unwrap();
        });
        assert_eq!(cache.indexes_built(), 2, "the new file's set, indexed once");
        assert_eq!(cache.rows_copied(), 2);
        assert!(cache.sets().building.is_empty(), "no mark is left behind");
        eprintln!("waits: {}", cache.waits());
    }

    /// After a usage flush the daemon calls `prewarm` (PERF-1 review): it copies the rows of
    /// the recent sets again for their new counters and keeps their indexes, after the text
    /// is out, so the next dictation copies nothing and still gets the request built whole.
    #[test]
    fn prewarm_after_a_usage_flush_leaves_the_next_request_nothing_to_copy() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("Biorouter", EntryTier::Public, None);
        let cache = DictionaryCache::new();
        let ask = |view: &StoreView| {
            cache.bias_request(
                view,
                &BiasTarget::public(),
                Standing::public(),
                true,
                "",
                BiasChannels::DEFAULT,
            )
        };
        let view = f.store.view().unwrap();
        ask(&view);
        assert_eq!((cache.rows_copied(), cache.indexes_built()), (1, 1));
        let id = view
            .entries()
            .iter()
            .find(|e| e.written == "EPAS1")
            .unwrap()
            .id
            .clone();
        let bump = UsageDelta {
            uses: 1,
            corrected_to: 0,
            last_used_at: Some(now()),
        };
        assert!(record_usage(
            &f.store,
            &[(id, UsageScope::Public, bump)],
            USAGE_FLUSH_BUDGET
        ));
        let flushed = f.store.view().unwrap();
        cache.prewarm(&flushed);
        assert_eq!(
            (cache.rows_copied(), cache.indexes_built()),
            (2, 1),
            "the rebuild copied the counters and kept the indexes"
        );
        let next = ask(&flushed);
        assert_eq!(
            (cache.rows_copied(), cache.indexes_built()),
            (2, 1),
            "the next request copies nothing"
        );
        let whole = DictionaryRequest::new(
            bias_set(&flushed, &BiasTarget::public(), Standing::public(), true),
            "",
            BiasChannels::DEFAULT,
        );
        assert_same_request(&next, &whole, "after the flush");
        let epas = next
            .eligible
            .entries()
            .iter()
            .find(|e| e.written == "EPAS1")
            .unwrap();
        assert_eq!(epas.uses, 1, "with the new counters");
    }

    /// Managed `dictionary: false` ("Managed policy keys"): `bias_set` gives the empty set
    /// whatever the store holds and whoever asks, so the dictation sends no prompt, builds
    /// no bonus table, corrects nothing and records no usage.
    #[test]
    fn a_forbidden_dictionary_gives_the_empty_set() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        f.add("PRIVATE1", EntryTier::Private, None);
        let view = f.store.view().unwrap();
        let forbidden = crate::managed::DictationPolicy::off().dictionary;
        assert!(!forbidden.is_allowed());
        let local = private_target(Some(ModelAffiliation::Local));
        let included = BiasTarget {
            person_included: true,
            ..BiasTarget::public()
        };
        let private = Standing::from_checked(true);
        for (target, standing, enforced) in [
            (&BiasTarget::public(), Standing::public(), true),
            (&local, private, true),
            (&included, private, true),
            (&BiasTarget::public(), Standing::public(), false),
        ] {
            let set = bias_set_under(&forbidden, &view, target, standing, enforced);
            assert!(set.is_empty(), "{target:?}");
            assert_eq!(set.scope(), BiasScope::Public);
            let allowed = bias_set_under(&Rule::Allowed, &view, target, standing, enforced);
            assert!(!allowed.is_empty(), "{target:?}");
        }
        let request = DictionaryRequest::new(
            bias_set_under(&forbidden, &view, &local, private, true),
            "",
            BiasChannels::BOTH,
        );
        assert!(request.prompt_terms().is_empty());
        assert!(request.bias_table().is_none());
        assert_eq!(request.correct("E-PAS-1 here", &[]).text, "E-PAS-1 here");
        let bumps = request.usage_bumps("EPAS1 and PRIVATE1", now());
        assert!(bumps.is_empty());
        assert!(!record_usage(&f.store, &bumps, USAGE_FLUSH_BUDGET));
        assert!(!f.store.usage_path().exists(), "no usage written");
        // This test binary's sandbox root holds no managed file, so `bias_set` reads the
        // dictionary as allowed.
        assert_eq!(
            bias_set(&view, &BiasTarget::public(), Standing::public(), true)
                .entries()
                .len(),
            1
        );
    }

    /// The tail is sent to the worker, never read by correction: the best term (the one
    /// the prompt puts nearest the audio) still corrects a truncated hearing of itself,
    /// given the prompt exactly as the service passes it (`prompt_terms`), and a piece of
    /// another prompt term is still left as it was heard.
    #[test]
    fn the_tail_does_not_reach_correction() {
        let f = Fixture::new();
        f.add("Modulite", EntryTier::Public, None);
        let request = DictionaryRequest::new(
            bias_set(
                &f.store.view().unwrap(),
                &BiasTarget::public(),
                Standing::public(),
                true,
            ),
            "",
            BiasChannels::DEFAULT,
        );
        let sent = request.prompt_terms();
        assert_eq!(sent, ["Modulite."], "the worker gets the tail");
        assert_eq!(
            request.correct("We saw Modulit today.", &sent).text,
            "We saw Modulite today."
        );
        assert_eq!(
            request.correct_piece("We saw Modulit today.", &sent),
            "We saw Modulite today."
        );

        let f = Fixture::new();
        f.add("ocrelizumab", EntryTier::Public, None);
        let request = DictionaryRequest::new(
            bias_set(
                &f.store.view().unwrap(),
                &BiasTarget::public(),
                Standing::public(),
                true,
            ),
            "",
            BiasChannels::DEFAULT,
        );
        let tailed = request.prompt_terms();
        assert_eq!(tailed, ["ocrelizumab."]);
        let untailed = vec!["ocrelizumab".to_string()];
        let heard = "She takes ocrelizuma now.";
        assert_eq!(
            request.correct(heard, &tailed).text,
            request.correct(heard, &untailed).text
        );
        assert_eq!(
            request.correct(heard, &tailed).text,
            "She takes ocrelizumab now."
        );

        // `Modulin` sorts first, so it is the best term and carries the tail; `Moduli` is
        // the head of `Modulite`, the other prompt term, and stays as it was heard.
        let f = Fixture::new();
        f.add("Modulite", EntryTier::Public, None);
        f.add("Modulin", EntryTier::Public, None);
        let request = DictionaryRequest::new(
            bias_set(
                &f.store.view().unwrap(),
                &BiasTarget::public(),
                Standing::public(),
                true,
            ),
            "",
            BiasChannels::DEFAULT,
        );
        let sent = request.prompt_terms();
        assert_eq!(sent, ["Modulite", "Modulin."]);
        assert_eq!(
            request.correct("piracy to Moduli today", &sent).text,
            "piracy to Moduli today"
        );
        // A best term that ends in punctuation gets no tail, and keeps its own period.
        let f = Fixture::new();
        f.add("Ph.D.", EntryTier::Public, None);
        let request = DictionaryRequest::new(
            bias_set(
                &f.store.view().unwrap(),
                &BiasTarget::public(),
                Standing::public(),
                true,
            ),
            "",
            BiasChannels::DEFAULT,
        );
        assert_eq!(request.prompt_terms(), ["Ph.D."]);
        assert_eq!(
            request.correction_prompt(&request.prompt_terms()).as_ref(),
            ["Ph.D."]
        );
    }

    /// The affiliation table of P-16 (O-35): a row's institutions against the target's
    /// model; a row that names two institutions reaches only a local model. Under each of
    /// the three mixing policies: `standard` and `strict` refuse a mismatch, and `open`
    /// lifts the institution check, as it does for extensions, so every private row
    /// reaches every private target. The tier rule holds under all three.
    #[test]
    fn the_affiliation_table() {
        use crate::privacy::mixing::{pin_for_test, MixingPolicy};

        let f = Fixture::new();
        f.add("NULLROW", EntryTier::Private, None);
        f.add("UCSFROW", EntryTier::Private, Some(&["ucsf"]));
        f.add("PAIRROW", EntryTier::Private, Some(&["ucsf", "stanford"]));
        f.add("UNKNOWNROW", EntryTier::Private, Some(&[]));
        let ucsf = Some(ModelAffiliation::institution(inst("ucsf")));
        let stanford = Some(ModelAffiliation::institution(inst("stanford")));
        let pair = ModelAffiliation::institutions([inst("ucsf"), inst("stanford")]);
        let private = Standing::from_checked(true);
        let rows = |names: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            v.sort();
            v
        };
        let every = rows(&["NULLROW", "UCSFROW", "PAIRROW", "UNKNOWNROW"]);
        for policy in [
            MixingPolicy::Open,
            MixingPolicy::Standard,
            MixingPolicy::Strict,
        ] {
            let _pin = pin_for_test(policy);
            let open = policy == MixingPolicy::Open;
            let unless_open = |names: &[&str]| if open { every.clone() } else { rows(names) };
            for (label, model, want) in [
                ("local", Some(ModelAffiliation::Local), every.clone()),
                ("ucsf", ucsf, unless_open(&["NULLROW", "UCSFROW"])),
                ("stanford", stanford, unless_open(&["NULLROW"])),
                ("stanford and ucsf", pair, unless_open(&["NULLROW"])),
                ("unstated", None, unless_open(&["NULLROW"])),
            ] {
                assert_eq!(
                    f.written(&private_target(model), private, true),
                    want,
                    "{policy:?} {label}"
                );
            }
            // The switch off lifts the check; so does the person including private terms.
            assert_eq!(
                f.written(&private_target(stanford), private, false),
                every,
                "{policy:?}"
            );
            let included = BiasTarget {
                private: true,
                affiliation: stanford,
                person_included: true,
            };
            assert_eq!(f.written(&included, private, true), every, "{policy:?}");
            // Without private standing no private row is eligible, whatever its
            // institutions and whatever the policy.
            for model in [ucsf, None, Some(ModelAffiliation::Local)] {
                assert!(
                    f.written(&private_target(model), Standing::public(), true)
                        .is_empty(),
                    "{policy:?} {model:?}"
                );
            }
            assert_eq!(
                f.written(&BiasTarget::public(), private, true),
                Vec::<String>::new(),
                "{policy:?}: a public target gets no private row"
            );
        }
    }

    /// Oracle rule 9: one private table per target affiliation, keyed by it.
    #[test]
    fn the_table_follows_the_target() {
        let f = Fixture::new();
        f.add("EPAS1", EntryTier::Public, None);
        let view = f.store.view().unwrap();
        let private = Standing::from_checked(true);
        let scope = |target: &BiasTarget, standing: Standing, enforced: bool| {
            bias_set(&view, target, standing, enforced).table().scope()
        };
        assert_eq!(scope(&BiasTarget::public(), private, true), "public");
        assert_eq!(
            scope(
                &private_target(Some(ModelAffiliation::Local)),
                private,
                true
            ),
            "private:local"
        );
        assert_eq!(
            scope(&private_target(None), private, true),
            "private:unstated"
        );
        assert_eq!(
            scope(
                &private_target(ModelAffiliation::institutions([inst("ucsf")])),
                private,
                true
            ),
            "private:ucsf"
        );
        assert_eq!(
            scope(&BiasTarget::public(), Standing::public(), false),
            "all"
        );
    }

    /// `eligible_entries_are_built_only_by_bias_set`: the leaf crate's constructor is
    /// called from this file alone, in production and in tests, everywhere in the tree.
    #[test]
    fn eligible_entries_are_built_only_by_bias_set() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut callers = Vec::new();
        let mut stack = vec![root.join("crates")];
        let mut files = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "target") {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files += 1;
                    let source = std::fs::read_to_string(&path).expect("readable");
                    // Rows become recognition entries only here too: `from_entry` takes
                    // a raw store row, so a caller elsewhere could rank or correct over
                    // rows `bias_set` never filtered. Matched with the type's name, so a
                    // helper of another type that shares the name (the decision log's
                    // `StoredRow::from_entry`) is not a caller, plus a `Self::` call in the
                    // file that defines the constructor.
                    let calls = [
                        ("EligibleEntries::new_filtered(", "pub fn new_filtered("),
                        ("RecognitionEntry::from_entry(", "pub fn from_entry("),
                    ]
                    .iter()
                    .map(|(call, definition)| {
                        let own = if source.contains(definition) {
                            let name = call.split("::").nth(1).unwrap_or(call);
                            source.matches(&format!("Self::{name}")).count()
                        } else {
                            0
                        };
                        source.matches(call).count() + own
                    })
                    .sum::<usize>();
                    if calls > 0 {
                        let relative = path
                            .strip_prefix(&root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .replace('\\', "/");
                        callers.push((relative, calls));
                    }
                }
            }
        }
        assert!(files > 400, "the walk read {files} files");
        // In this file: the two calls in `bias_set` and the two patterns above.
        assert_eq!(
            callers,
            [("crates/biorouter/src/dictation/bias.rs".to_string(), 4)],
            "EligibleEntries::new_filtered or RecognitionEntry::from_entry is called outside \
             bias_set"
        );
    }
}
