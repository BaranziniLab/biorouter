//! What a correction the person made to dictated text does to the store
//! (`docs/dictation/dictionary.md`, "Effects on the dictionary", DD-58, O-7), for
//! `POST /dictionary/observations` (PR-16) and, from PR-26, the TUI and the classic REPL.
//!
//! | Observation | Effect |
//! |---|---|
//! | A correction whose `to` is a row's written form (or step 3 key) in the stamp tier | `from` joins that row's misrecognitions with `count` 1, or its `count` rises; the row's `corrected_to` rises by 1 |
//! | A correction to a term with no row there | a new `suggested` row, `source: correction`, with `from` as its misrecognition; the tier's tombstones block it |
//! | A private stamp on a term only a public row holds | an active private twin, with the public row's written form, kind and `match`; the public row is never changed |
//! | A correction whose `from` is a single common English word | no misrecognition anywhere; only `corrected_to` rises |
//! | Two `rejected` sightings of one (entry, `from`), from distinct records | a learned misrecognition `from` is removed, or a `fuzzy` entry becomes `exact` |
//!
//! Each observation counts once: the store keeps a digest of (`record_id`, `from`, `to`)
//! per tier ([`Sighting`]), so a resend or the same pair posted again changes nothing.
//! The caller decides the stamp (its chat's tier and institutions) and whether a private
//! row's institutions admit the chat's model (`admits`); this module never reads either.
//! A correction never moves a row between tiers, and a rejection reaches only a row of the
//! stamp tier, so a private chat never changes a public row (DD-35).

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::ops::{AddOutcome, NewTerm};
use super::schema::{
    Entry, EntryId, EntrySource, EntryStatus, EntryTier, MatchPolicy, Misrecognition,
    RowAffiliation, Sighting, StoreFile, MAX_SIGHTINGS_PER_TIER,
};
use super::validate;
use crate::recognize::{key, words};

/// What the person did to a dictated span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationKind {
    /// The person replaced `from` (what dictation wrote) with `to`.
    Correction,
    /// The person reverted a change dictation made: `from` is the transcript's text again,
    /// `to` the change it undid, and `entry_id` the row that made it.
    Rejected,
}

/// One observation, as the route or the terminal has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The dictation the span came from: its stream id.
    pub record_id: String,
    pub kind: ObservationKind,
    pub from: String,
    pub to: String,
    pub entry_id: Option<EntryId>,
}

/// The writer's stamp for an observation: the tier its chat answers (`chat_reach`, Private
/// unless Public), the institutions a private row it creates records, and the chat itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationStamp {
    pub tier: EntryTier,
    /// Kept only on a private row this observation creates; `None` on a public stamp.
    pub affiliation: Option<RowAffiliation>,
    /// The chat the correction was made in, recorded as `suggested_in` on a row it creates.
    pub session_id: Option<String>,
}

/// What an observation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// Nothing changed: already seen, a term that cannot be stored, a row whose
    /// institutions do not admit the chat's model, a tombstone, or full suggestions.
    Nothing,
    /// The row the person corrected to: its `corrected_to` rises by one in the usage file.
    CorrectedTo(EntryId),
    /// A rejection was counted.
    Rejected,
}

/// The first 32 hexadecimal digits of the SHA-256 of `parts`, each closed by a NUL, which
/// no validated term holds.
fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let hash = hasher.finalize();
    hash.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// A single word the English word list calls ordinary ("Effects on the dictionary").
fn single_common_word(text: &str) -> bool {
    !text.contains(char::is_whitespace) && words::is_common(text)
}

impl StoreFile {
    /// Apply one observation under `stamp`. `admits` answers whether a private row's
    /// institutions all admit the model of the chat the observation came from; it is asked
    /// only of an existing row the observation would change. `at` is the write's time.
    pub fn observe(
        &mut self,
        observation: &Observation,
        stamp: &ObservationStamp,
        admits: &dyn Fn(&Entry) -> bool,
        at: DateTime<Utc>,
    ) -> Observed {
        let key = digest(&[&observation.record_id, &observation.from, &observation.to]);
        if self
            .sightings
            .iter()
            .any(|s| s.tier == stamp.tier && s.key == key)
        {
            return Observed::Nothing;
        }
        let (Ok(from), Ok(to)) = (
            validate::term(&observation.from),
            validate::term(&observation.to),
        ) else {
            return Observed::Nothing;
        };
        if from == to {
            return Observed::Nothing;
        }
        let (observed, rejected) = match observation.kind {
            ObservationKind::Correction => {
                (self.observe_correction(&from, &to, stamp, admits, at), None)
            }
            ObservationKind::Rejected => {
                let Some(id) = observation.entry_id.as_ref() else {
                    return Observed::Nothing;
                };
                let subject = digest(&[id.as_str(), &from]);
                match self.index_in(id, stamp.tier, admits) {
                    Some(index) => {
                        let seen_before = self
                            .sightings
                            .iter()
                            .filter(|s| {
                                s.tier == stamp.tier && s.rejected.as_ref() == Some(&subject)
                            })
                            .count();
                        if seen_before >= 1 {
                            self.reject_change(index, &from, at);
                        }
                        (Observed::Rejected, Some(subject))
                    }
                    None => (Observed::Nothing, None),
                }
            }
        };
        if observed != Observed::Nothing {
            self.sight(Sighting {
                key,
                tier: stamp.tier,
                rejected,
                at,
            });
        }
        observed
    }

    /// The row `id` when it is in `tier` and, if private, its institutions admit the chat.
    fn index_in(
        &self,
        id: &EntryId,
        tier: EntryTier,
        admits: &dyn Fn(&Entry) -> bool,
    ) -> Option<usize> {
        let index = self.entries.iter().position(|e| &e.id == id)?;
        let entry = &self.entries[index];
        (entry.tier == tier && (!tier.is_private() || admits(entry))).then_some(index)
    }

    /// The row of `tier` a correction to `to` lands on: the written form exactly, else the
    /// row whose written form has the same step 3 key (`E-PAS-1` for `EPAS1`), the lowest
    /// written form first when several do.
    fn correction_target(&self, to: &str, tier: EntryTier) -> Option<usize> {
        if let Some(exact) = self
            .entries
            .iter()
            .position(|e| e.tier == tier && e.written == to)
        {
            return Some(exact);
        }
        let wanted = key::form_key(to).text;
        if wanted.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.tier == tier && key::form_key(&e.written).text == wanted)
            .min_by(|(_, a), (_, b)| a.written.cmp(&b.written))
            .map(|(index, _)| index)
    }

    fn observe_correction(
        &mut self,
        from: &str,
        to: &str,
        stamp: &ObservationStamp,
        admits: &dyn Fn(&Entry) -> bool,
        at: DateTime<Utc>,
    ) -> Observed {
        let common = single_common_word(from);
        if let Some(index) = self.correction_target(to, stamp.tier) {
            let entry = &self.entries[index];
            if entry.tier.is_private() && !admits(entry) {
                return Observed::Nothing;
            }
            let id = entry.id.clone();
            if !common {
                self.sight_misrecognition(index, from, at);
            }
            return Observed::CorrectedTo(id);
        }
        // A private stamp on a term only a public row holds: an active private twin with
        // the public row's spelling, kind and match, and the public row left as it is.
        let public_twin = stamp
            .tier
            .is_private()
            .then(|| self.correction_target(to, EntryTier::Public))
            .flatten();
        let term = match public_twin {
            Some(public) => {
                let row = &self.entries[public];
                NewTerm {
                    source: EntrySource::Correction,
                    status: EntryStatus::Active,
                    kind: row.kind,
                    match_policy: Some(row.match_policy),
                    ..NewTerm::person(row.written.clone(), EntryTier::Private)
                }
            }
            None => NewTerm {
                source: EntrySource::Correction,
                status: EntryStatus::Suggested,
                ..NewTerm::person(to, stamp.tier)
            },
        };
        let term = NewTerm {
            misrecognitions: if common {
                Vec::new()
            } else {
                vec![from.to_string()]
            },
            suggested_in: stamp.session_id.clone(),
            affiliation: stamp.affiliation.clone(),
            ..term
        };
        match self.add_with(term, None, at) {
            Ok(AddOutcome::Created { id, .. }) => Observed::CorrectedTo(id),
            Ok(AddOutcome::Exists { id }) => {
                // A row with the same letters in the tier (the fold key dedupe of a writer
                // that is not the person): the same as finding it above.
                let Some(index) = self.index_in(&id, stamp.tier, admits) else {
                    return Observed::Nothing;
                };
                if !common {
                    self.sight_misrecognition(index, from, at);
                }
                Observed::CorrectedTo(id)
            }
            Ok(AddOutcome::Skipped) | Err(_) => Observed::Nothing,
        }
    }

    /// One more sighting of `from` on a row: count 1 when new (correction uses it from 2,
    /// O-7), else one more; a learned row also counts the sighting in `seen`.
    fn sight_misrecognition(&mut self, index: usize, from: &str, at: DateTime<Utc>) {
        let entry = &mut self.entries[index];
        if entry.written == from || entry.spoken_forms.iter().any(|f| f == from) {
            return;
        }
        let room = entry.misrecognitions.len() < validate::MAX_MISRECOGNITIONS;
        match entry.misrecognitions.iter_mut().find(|m| m.text == from) {
            Some(known) => {
                known.count = known.count.saturating_add(1);
                known.last_seen = at;
            }
            None if room => {
                entry.misrecognitions.push(Misrecognition {
                    text: from.to_string(),
                    count: 1,
                    last_seen: at,
                });
            }
            None => return,
        }
        if entry.source == EntrySource::Correction {
            entry.seen = entry.seen.saturating_add(1);
        }
        entry.touch(at);
    }

    /// The second distinct rejection of one change: a learned misrecognition goes, else a
    /// fuzzy row becomes exact.
    fn reject_change(&mut self, index: usize, from: &str, at: DateTime<Utc>) {
        let entry = &mut self.entries[index];
        if let Some(position) = entry.misrecognitions.iter().position(|m| m.text == from) {
            entry.misrecognitions.remove(position);
            entry.touch(at);
        } else if entry.match_policy == MatchPolicy::Fuzzy {
            entry.match_policy = MatchPolicy::Exact;
            entry.touch(at);
        }
    }

    /// Keep a sighting, dropping the oldest of its tier beyond the cap.
    fn sight(&mut self, sighting: Sighting) {
        let tier = sighting.tier;
        self.sightings.push(sighting);
        if self.sightings.iter().filter(|s| s.tier == tier).count() > MAX_SIGHTINGS_PER_TIER {
            if let Some(oldest) = self
                .sightings
                .iter()
                .enumerate()
                .filter(|(_, s)| s.tier == tier)
                .min_by(|(_, a), (_, b)| a.at.cmp(&b.at).then(a.key.cmp(&b.key)))
                .map(|(i, _)| i)
            {
                self.sightings.remove(oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ops::Reach;
    use crate::store::schema::now;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + seconds, 0).unwrap()
    }

    fn correction(record: &str, from: &str, to: &str) -> Observation {
        Observation {
            record_id: record.into(),
            kind: ObservationKind::Correction,
            from: from.into(),
            to: to.into(),
            entry_id: None,
        }
    }

    fn rejected(record: &str, from: &str, to: &str, entry: &EntryId) -> Observation {
        Observation {
            kind: ObservationKind::Rejected,
            entry_id: Some(entry.clone()),
            ..correction(record, from, to)
        }
    }

    fn stamp(tier: EntryTier) -> ObservationStamp {
        ObservationStamp {
            tier,
            affiliation: tier.is_private().then(|| RowAffiliation {
                institutions: vec!["ucsf".into()],
            }),
            session_id: Some("20261106_1".into()),
        }
    }

    fn everything(_: &Entry) -> bool {
        true
    }

    fn add(file: &mut StoreFile, term: NewTerm) -> EntryId {
        match file.add(term, Reach::All, now()).unwrap() {
            AddOutcome::Created { id, .. } => id,
            other => panic!("{other:?}"),
        }
    }

    /// O-7: the first sighting adds the misrecognition at count 1, a resend of the same
    /// (record, from, to) changes nothing, and a second record raises it to 2, the count at
    /// which correction uses it.
    #[test]
    fn a_correction_counts_once_per_record() {
        let mut file = StoreFile::default();
        let epas = add(&mut file, NewTerm::person("EPAS1", EntryTier::Public));
        let public = stamp(EntryTier::Public);
        let first = correction("dct_stream_1", "a pass one", "EPAS1");
        assert_eq!(
            file.observe(&first, &public, &everything, at(1)),
            Observed::CorrectedTo(epas.clone())
        );
        let once = file.clone();
        assert_eq!(
            file.observe(&first, &public, &everything, at(2)),
            Observed::Nothing,
            "a resend"
        );
        assert_eq!(file, once, "byte identical after a resend");
        let row = file.find(&epas).unwrap();
        assert_eq!(row.misrecognitions[0].count, 1);
        file.observe(
            &correction("dct_stream_2", "a pass one", "EPAS1"),
            &public,
            &everything,
            at(3),
        );
        assert_eq!(file.find(&epas).unwrap().misrecognitions[0].count, 2);
        assert_eq!(file.sightings.len(), 2);
        let text = String::from_utf8(file.to_bytes()).unwrap();
        assert!(!text.contains("a pass one\",\"tier"), "{text}");
        assert!(
            !text.contains("dct_stream_1"),
            "a sighting holds digests only: {text}"
        );
    }

    /// The step 3 key reaches a row whose spelling differs only in form, and a single
    /// common word as `from` stores no misrecognition, only the corrected_to bump.
    #[test]
    fn keys_and_common_words() {
        let mut file = StoreFile::default();
        let epas = add(&mut file, NewTerm::person("EPAS-1", EntryTier::Public));
        let public = stamp(EntryTier::Public);
        assert_eq!(
            file.observe(
                &correction("r1", "the", "EPAS1"),
                &public,
                &everything,
                at(1)
            ),
            Observed::CorrectedTo(epas.clone())
        );
        assert!(file.find(&epas).unwrap().misrecognitions.is_empty());
        assert_eq!(file.find(&epas).unwrap().rev, 1, "the row did not change");
    }

    /// A correction to a term with no row is a suggestion from `correction` carrying the
    /// chat it came from; a tombstone in the stamp tier blocks it.
    #[test]
    fn a_correction_to_a_new_term_is_a_suggestion_tombstones_block() {
        let mut file = StoreFile::default();
        let public = stamp(EntryTier::Public);
        let Observed::CorrectedTo(id) = file.observe(
            &correction("r1", "spoke", "SPOKE"),
            &public,
            &everything,
            at(1),
        ) else {
            panic!("a new row");
        };
        let row = file.find(&id).unwrap().clone();
        assert_eq!(
            (row.status, row.source, row.tier),
            (
                EntryStatus::Suggested,
                EntrySource::Correction,
                EntryTier::Public
            )
        );
        assert_eq!(row.suggested_in.as_deref(), Some("20261106_1"));
        assert_eq!(row.affiliation, None, "a public row carries none");
        file.remove(&id, at(2)).unwrap();
        assert_eq!(
            file.observe(
                &correction("r2", "spoke", "SPOKE"),
                &public,
                &everything,
                at(3)
            ),
            Observed::Nothing,
            "the person deleted it"
        );
    }

    /// DD-35: a private stamp never changes a public row. It lands in an active private
    /// twin with the public row's spelling, kind and match, stamped with the chat's
    /// institutions.
    #[test]
    fn a_private_correction_of_a_public_term_lands_in_a_private_twin() {
        let mut file = StoreFile::default();
        let public = add(
            &mut file,
            NewTerm {
                kind: Some(crate::store::schema::TermKind::GeneSymbol),
                ..NewTerm::person("EPAS1", EntryTier::Public)
            },
        );
        let before = file.find(&public).unwrap().clone();
        let private = stamp(EntryTier::Private);
        let Observed::CorrectedTo(twin) = file.observe(
            &correction("r1", "a pass one", "EPAS1"),
            &private,
            &everything,
            at(1),
        ) else {
            panic!("a twin");
        };
        assert_ne!(twin, public);
        assert_eq!(
            file.find(&public).unwrap(),
            &before,
            "the public row is untouched"
        );
        let row = file.find(&twin).unwrap();
        assert_eq!(
            (row.tier, row.status),
            (EntryTier::Private, EntryStatus::Active)
        );
        assert_eq!(row.kind, before.kind);
        assert_eq!(row.match_policy, before.match_policy);
        assert_eq!(row.misrecognitions[0].text, "a pass one");
        assert_eq!(
            row.affiliation,
            Some(RowAffiliation {
                institutions: vec!["ucsf".into()]
            })
        );
    }

    /// An observation whose chat's model a private row's institutions do not admit leaves
    /// that row as it was, and records nothing, so the store is byte identical.
    #[test]
    fn a_row_that_does_not_admit_the_chat_is_left_alone() {
        let mut file = StoreFile::default();
        let id = add(&mut file, NewTerm::person("EPAS1", EntryTier::Private));
        let before = file.to_bytes();
        let refuses = |_: &Entry| false;
        let private = stamp(EntryTier::Private);
        for observation in [
            correction("r1", "a pass one", "EPAS1"),
            rejected("r1", "a pass one", "EPAS1", &id),
        ] {
            assert_eq!(
                file.observe(&observation, &private, &refuses, at(1)),
                Observed::Nothing
            );
        }
        assert_eq!(file.to_bytes(), before);
    }

    /// Two rejections of one change from distinct records remove a learned misrecognition,
    /// or make a fuzzy row exact; one, or the same one twice, changes nothing; a public
    /// stamp never reaches a private row.
    #[test]
    fn two_rejections_undo_a_learned_change() {
        let mut file = StoreFile::default();
        let spoke = add(
            &mut file,
            NewTerm {
                misrecognitions: vec!["spoke".into()],
                ..NewTerm::person("SPOKE", EntryTier::Private)
            },
        );
        let private = stamp(EntryTier::Private);
        let first = rejected("r1", "spoke", "SPOKE", &spoke);
        assert_eq!(
            file.observe(&first, &private, &everything, at(1)),
            Observed::Rejected
        );
        assert_eq!(
            file.observe(&first, &private, &everything, at(2)),
            Observed::Nothing
        );
        assert_eq!(file.find(&spoke).unwrap().misrecognitions.len(), 1);
        assert_eq!(
            file.observe(
                &rejected("r2", "spoke", "SPOKE", &spoke),
                &stamp(EntryTier::Public),
                &everything,
                at(3)
            ),
            Observed::Nothing,
            "a public stamp never reaches a private row"
        );
        file.observe(
            &rejected("r3", "spoke", "SPOKE", &spoke),
            &private,
            &everything,
            at(4),
        );
        assert!(file.find(&spoke).unwrap().misrecognitions.is_empty());

        let ventolin = add(&mut file, NewTerm::person("Ventolin", EntryTier::Private));
        assert_eq!(
            file.find(&ventolin).unwrap().match_policy,
            MatchPolicy::Fuzzy
        );
        for record in ["r4", "r5"] {
            file.observe(
                &rejected(record, "ventilin", "Ventolin", &ventolin),
                &private,
                &everything,
                at(5),
            );
        }
        assert_eq!(
            file.find(&ventolin).unwrap().match_policy,
            MatchPolicy::Exact
        );
    }

    /// Terms a row cannot hold are dropped without a trace.
    #[test]
    fn unstorable_pairs_are_dropped() {
        let mut file = StoreFile::default();
        let before = file.clone();
        for (from, to) in [
            ("see the link", "https://example.org"),
            ("a\nb", "EPAS1"),
            ("EPAS1", "EPAS1"),
            ("", "EPAS1"),
        ] {
            assert_eq!(
                file.observe(
                    &correction("r", from, to),
                    &stamp(EntryTier::Public),
                    &everything,
                    at(1)
                ),
                Observed::Nothing,
                "{from} -> {to}"
            );
        }
        assert_eq!(file, before);
    }

    #[test]
    fn sightings_are_capped_per_tier() {
        let mut file = StoreFile::default();
        add(&mut file, NewTerm::person("EPAS1", EntryTier::Public));
        let public = stamp(EntryTier::Public);
        for i in 0..MAX_SIGHTINGS_PER_TIER + 5 {
            file.observe(
                &correction(&format!("r{i}"), "the", "EPAS1"),
                &public,
                &everything,
                at(i as i64),
            );
        }
        assert_eq!(file.sightings.len(), MAX_SIGHTINGS_PER_TIER);
        assert!(StoreFile::parse(&file.to_bytes()).is_some());
    }

    /// What a full sightings list costs the parse a dictation's `view()` pays once after
    /// each change to the store: 7,800 rows, the size `measure_the_store_at_the_caps`
    /// uses, with and without 1,000 sightings in each tier. A timing run, by hand:
    /// `cargo test -p biorouter-dictation --lib store::observe::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing run, by hand"]
    fn measure_the_sightings_cost() {
        use std::time::{Duration, Instant};
        let mut file = StoreFile::default();
        let row = add(&mut file, NewTerm::person("QZV", EntryTier::Public));
        let template = file.find(&row).unwrap().clone();
        file.entries.clear();
        for i in 0..7_800 {
            file.entries.push(Entry {
                id: EntryId::random(),
                written: format!("QZV{i}X"),
                tier: if i % 2 == 0 {
                    EntryTier::Public
                } else {
                    EntryTier::Private
                },
                spoken_forms: vec![format!("term {i} spoken")],
                misrecognitions: vec![Misrecognition {
                    text: format!("term {i} heard"),
                    count: 2,
                    last_seen: at(0),
                }],
                note: Some("a note of a few words".into()),
                ..template.clone()
            });
        }
        let median = |bytes: &[u8]| -> Duration {
            let mut samples: Vec<Duration> = (0..9)
                .map(|_| {
                    let started = Instant::now();
                    assert!(StoreFile::parse(bytes).is_some());
                    started.elapsed()
                })
                .collect();
            samples.sort();
            samples[samples.len() / 2]
        };
        let without = file.to_bytes();
        for tier in [EntryTier::Public, EntryTier::Private] {
            for i in 0..MAX_SIGHTINGS_PER_TIER {
                file.sight(Sighting {
                    key: digest(&[&format!("r{i}"), "from", "to"]),
                    tier,
                    rejected: (i % 4 == 0).then(|| digest(&["dct_x", "from"])),
                    at: at(i as i64),
                });
            }
        }
        let with = file.to_bytes();
        println!(
            "7,800 rows: {} bytes, parse {:?}; with 2,000 sightings: {} bytes (+{}), parse {:?}",
            without.len(),
            median(&without),
            with.len(),
            with.len() - without.len(),
            median(&with)
        );
    }
}
