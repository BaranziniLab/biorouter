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
//! dictation starts so the path from stop to text pays for none of them. After the text
//! is out, [`DictionaryRequest::usage_bumps`] says which rows it used and
//! [`record_usage`] writes them to `usage.json` ("Usage").

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use biorouter_dictation::protocol::{BonusParams, SetBias};
use biorouter_dictation::recognize::correct::{Change, Corrected, Corrector};
use biorouter_dictation::recognize::prompt;
use biorouter_dictation::recognize::select::{self, RequestPlan};
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
    if !dictionary.is_allowed() {
        return EligibleEntries::empty();
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
    let usage = table.usage_scope();
    let rows: Vec<RecognitionEntry> = view
        .entries()
        .iter()
        .filter(|e| RecognitionEntry::is_recognizable(e))
        .filter(|e| privacy::visible_to(reach, classification_of(e.tier)))
        .filter(|e| target.person_included || admits(e, target, reach, enforced))
        .map(|e| RecognitionEntry::from_entry(e, view.usage(&e.id, usage)))
        .collect();
    EligibleEntries::new_filtered(rows, table, scope, view.generation())
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
                    // rows `bias_set` never filtered.
                    let calls = [
                        ("new_filtered(", "pub fn new_filtered("),
                        ("from_entry(", "pub fn from_entry("),
                    ]
                    .iter()
                    .map(|(call, definition)| {
                        source.matches(call).count() - source.matches(definition).count()
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
