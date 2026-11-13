//! What a write does to the store file: adds with their tier, tombstone and cap rules,
//! edits, deletes, accepts and the settings. Every function here runs inside
//! [`super::DictionaryStore::write`], on a file read fresh under the lock.
//!
//! Rows are unique on (`written` exact after NFC, `tier`), so the same term may exist
//! once as public and once as private (twins, DD-35), and a private write never changes
//! a public row. Caps and tombstones count only the tier being written (DD-40, DD-41),
//! so a public caller learns nothing about private rows from them.

use chrono::{DateTime, Utc};

use super::fold::fold_key;
use super::learning::{chat_digest, MAX_REQUIRED_CHATS};
use super::schema::{
    BiasPolicy, Entry, EntryId, EntrySource, EntryStatus, EntryTier, MatchPolicy, Misrecognition,
    RowAffiliation, StoreFile, TermKind, Tombstone,
};
use super::validate::{self, ValidationCode};
use super::{CapKind, StoreError};

/// Active rows per tier: beyond it a person's add is refused and any other writer's
/// term lands suggested.
pub const MAX_ACTIVE_PER_TIER: usize = 5_000;
/// Pending suggestions per tier: beyond it the oldest `auto` row seen once is evicted,
/// and with none to evict the term is not recorded.
pub const MAX_SUGGESTIONS_PER_TIER: usize = 200;
/// Agent rows the person has not kept, per tier: beyond it agent terms land suggested.
pub const MAX_UNREVIEWED_AGENT_PER_TIER: usize = 200;
/// Tombstones per tier: beyond it the oldest is dropped.
pub const MAX_TOMBSTONES_PER_TIER: usize = 5_000;

/// The tiers a caller may list (DD-37): public rows to everyone, private rows only to a
/// caller with private standing. It decides what the "same letters" warning and an
/// import plan may consult.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Public,
    All,
}

impl Reach {
    pub fn lists(self, tier: EntryTier) -> bool {
        self == Self::All || tier == EntryTier::Public
    }
}

/// A term to add, as its writer has it. Every text field is validated by
/// [`StoreFile::add`]; the tier and affiliation are the writer's stamp, never a
/// client's.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTerm {
    pub written: String,
    pub tier: EntryTier,
    pub source: EntrySource,
    /// The status the writer asks for; caps may turn `Active` into `Suggested`.
    pub status: EntryStatus,
    pub spoken_forms: Vec<String>,
    pub misrecognitions: Vec<String>,
    pub kind: Option<TermKind>,
    /// `None` takes the default for the kind.
    pub match_policy: Option<MatchPolicy>,
    pub bias: BiasPolicy,
    pub context_cues: Vec<String>,
    pub note: Option<String>,
    pub reason: Option<String>,
    pub suggested_in: Option<String>,
    /// Kept on a private row; a public row never carries one.
    pub affiliation: Option<RowAffiliation>,
}

impl NewTerm {
    /// A term a person adds by hand: active, from `user`, with no institution (O-36).
    pub fn person(written: impl Into<String>, tier: EntryTier) -> Self {
        Self {
            written: written.into(),
            tier,
            source: EntrySource::User,
            status: EntryStatus::Active,
            spoken_forms: Vec::new(),
            misrecognitions: Vec::new(),
            kind: None,
            match_policy: None,
            bias: BiasPolicy::Auto,
            context_cues: Vec::new(),
            note: None,
            reason: None,
            suggested_in: None,
            affiliation: None,
        }
    }
}

/// What an add did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    /// A new row. `same_letters` names a row the caller can list whose letters fold to
    /// the same key (MSG-SAME-LETTERS); it is a warning, never a merge.
    Created {
        id: EntryId,
        status: EntryStatus,
        same_letters: Option<String>,
    },
    /// The row already exists in the stamp tier; nothing changed.
    Exists { id: EntryId },
    /// Nothing was written: a tombstone in the stamp tier stops a writer that is not
    /// the person, or the suggestions were full with nothing to evict.
    Skipped,
}

/// The fields an edit may change; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntryEdit {
    pub written: Option<String>,
    pub spoken_forms: Option<Vec<String>>,
    pub kind: Option<Option<TermKind>>,
    pub note: Option<Option<String>>,
    pub match_policy: Option<MatchPolicy>,
    pub bias: Option<BiasPolicy>,
    pub context_cues: Option<Vec<String>>,
}

fn cues(raw: &[String]) -> Result<Vec<String>, ValidationCode> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for cue in raw {
        let cue = validate::cue(cue)?;
        if !out.contains(&cue) {
            out.push(cue);
        }
    }
    if out.len() > validate::MAX_CONTEXT_CUES {
        return Err(ValidationCode::TooManyForms);
    }
    Ok(out)
}

fn optional(
    raw: Option<&str>,
    check: fn(&str) -> Result<Option<String>, ValidationCode>,
) -> Result<Option<String>, ValidationCode> {
    raw.map_or(Ok(None), check)
}

fn unreviewed_agent(entry: &Entry) -> bool {
    entry.source == EntrySource::Agent && entry.status == EntryStatus::Active && !entry.reviewed
}

/// The `match` a row takes when an edit changes its spelling or its kind and names no
/// `match` of its own ("Defaults by kind"). A stored value equal to the old form's
/// default was never chosen, so it follows the new form's default; a `fuzzy` that the
/// new form could never default to becomes `exact`, because short symbols and letter
/// and digit shapes are never fuzzy; anything else (`prompt_only`, or an `exact` set on a
/// long name) stays. An agent row stays `exact` (DD-54).
fn match_after_edit(before: &Entry, after: &Entry) -> MatchPolicy {
    if after.source == EntrySource::Agent {
        return MatchPolicy::Exact;
    }
    let old_default = MatchPolicy::default_for(before.kind, &before.written);
    let new_default = MatchPolicy::default_for(after.kind, &after.written);
    match before.match_policy {
        stored if stored == old_default => new_default,
        MatchPolicy::Fuzzy if new_default == MatchPolicy::Exact => MatchPolicy::Exact,
        stored => stored,
    }
}

impl StoreFile {
    /// The row with this id.
    pub fn find(&self, id: &EntryId) -> Option<&Entry> {
        self.entries.iter().find(|entry| &entry.id == id)
    }

    fn index_of(&self, id: &EntryId) -> Result<usize, StoreError> {
        self.entries
            .iter()
            .position(|entry| &entry.id == id)
            .ok_or(StoreError::NotFound)
    }

    /// The rows the caller can list whose written form is exactly `written` after NFC:
    /// one, or twins.
    pub fn rows_written(&self, written: &str, reach: Reach) -> Vec<&Entry> {
        let written = validate::normalize(written);
        self.entries
            .iter()
            .filter(|entry| reach.lists(entry.tier) && entry.written == written)
            .collect()
    }

    /// How many rows of `tier` satisfy `which`.
    pub fn count_in(&self, tier: EntryTier, which: impl Fn(&Entry) -> bool) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.tier == tier && which(entry))
            .count()
    }

    /// Whether a tombstone in `tier` holds this fold key.
    pub fn tombstoned(&self, fold: &str, tier: EntryTier) -> bool {
        self.tombstones
            .iter()
            .any(|stone| stone.tier == tier && stone.fold_key == fold)
    }

    /// Count a chat a chat extractor suggestion was found in. A word counts once per chat
    /// (`seen_in` keeps a digest of each), and it is listed for review (`seen` 2) once it
    /// has been found in as many different chats as the person's own decisions ask for
    /// ([`super::ChatLearning::required_chats`]: two, or more when they dismiss most). A
    /// listed suggestion, a row that is not a suggestion, and a tombstoned word are left
    /// alone, so a dismissed word is never suggested again.
    pub fn observe_auto_suggestion(
        &mut self,
        written: &str,
        tier: EntryTier,
        session_id: &str,
        affiliation: Option<RowAffiliation>,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let written = validate::term(written)?;
        let fold = fold_key(&written);
        if self.tombstoned(&fold, tier) {
            return Ok(());
        }
        // DD-57: a term the caller can already list is not proposed again. A private
        // caller lists the public rows too (every row but an `auto` row seen once), so a
        // term already public gets no private twin. A public caller lists no private
        // row, so a private row changes nothing for it.
        if tier.is_private()
            && self.entries.iter().any(|row| {
                row.tier != tier
                    && row.fold_key() == fold
                    && !(row.source == EntrySource::Auto && row.seen < 2)
            })
        {
            return Ok(());
        }
        let required = self.chat_learning.required_chats() as usize;
        let chat = chat_digest(session_id);
        if let Some(row) = self
            .entries
            .iter_mut()
            .find(|row| row.tier == tier && row.fold_key() == fold)
        {
            if row.source != EntrySource::Auto
                || row.status != EntryStatus::Suggested
                || row.seen >= 2
            {
                return Ok(());
            }
            // A suggestion written before chats were counted names only its latest chat.
            if row.seen_in.is_empty() {
                if let Some(first) = row.suggested_in.as_deref() {
                    row.seen_in.push(chat_digest(first));
                }
            }
            if row.seen_in.contains(&chat) {
                return Ok(());
            }
            if row.seen_in.len() < MAX_REQUIRED_CHATS as usize {
                row.seen_in.push(chat);
            }
            row.suggested_in = Some(session_id.to_owned());
            if row.seen_in.len() >= required {
                row.seen = 2;
            }
            row.touch(at);
            return Ok(());
        }
        let mut term = NewTerm::person(written, tier);
        term.source = EntrySource::Auto;
        term.status = EntryStatus::Suggested;
        term.suggested_in = Some(session_id.to_owned());
        term.affiliation = if tier.is_private() { affiliation } else { None };
        if let AddOutcome::Created { id, .. } = self.add(term, Reach::All, at)? {
            if let Some(row) = self.entries.iter_mut().find(|row| row.id == id) {
                row.seen_in = vec![chat];
            }
        }
        Ok(())
    }

    /// Whether a row is a chat extractor suggestion the person can see, so a decision on
    /// it measures how well suggestions suit them.
    fn listed_chat_suggestion(entry: &Entry) -> bool {
        entry.source == EntrySource::Auto && entry.status == EntryStatus::Suggested && entry.seen >= 2
    }

    /// After the person's decisions lowered the chats a word needs, list the suggestions
    /// that now have enough. Raising it never hides one already listed.
    fn list_suggestions_that_now_qualify(&mut self, at: DateTime<Utc>) {
        let required = self.chat_learning.required_chats() as usize;
        for row in self.entries.iter_mut() {
            if row.source == EntrySource::Auto
                && row.status == EntryStatus::Suggested
                && row.seen < 2
                && row.seen_in.len() >= required
            {
                row.seen = 2;
                row.touch(at);
            }
        }
    }

    /// Record one decision on a listed chat suggestion, and list what the new threshold
    /// admits.
    fn record_chat_decision(&mut self, kept: bool, at: DateTime<Utc>) {
        self.chat_learning.record(kept);
        self.list_suggestions_that_now_qualify(at);
    }

    /// Dismiss the suggestions a person chose from an earlier read: each still here and
    /// still a suggestion is deleted with its tombstone, so it is not suggested again, and
    /// each listed chat suggestion among them counts as a **Dismiss** in
    /// [`super::ChatLearning`]. The rows dismissed.
    pub fn dismiss_suggestions(&mut self, ids: &[EntryId], at: DateTime<Utc>) -> Vec<Entry> {
        let removed = self.remove_confirmed(ids, |e| e.status == EntryStatus::Suggested, at);
        let measured = removed
            .iter()
            .filter(|entry| Self::listed_chat_suggestion(entry))
            .count();
        for _ in 0..measured {
            self.record_chat_decision(false, at);
        }
        removed
    }

    /// Add a term ("Uniqueness", "Caps per tier", "Who stamps the tier").
    ///
    /// A person's term (`user`, `import`) is refused beyond 5,000 active rows in its tier,
    /// and once admitted clears a tombstone for its key there. Any other writer's term is stopped by
    /// that tombstone, is deduped by fold key in its tier, lands suggested beyond the
    /// active or the unreviewed agent cap, and takes the place of the oldest `auto` row
    /// seen once when the suggestions are full, or is not recorded.
    pub fn add(
        &mut self,
        term: NewTerm,
        reach: Reach,
        at: DateTime<Utc>,
    ) -> Result<AddOutcome, StoreError> {
        self.add_with(term, Some(reach), at)
    }

    /// [`Self::add`]; `warn` is the reach the "same letters" warning consults, or `None`
    /// for an import, which reports no warning and so skips folding every row per row
    /// (an import of 1,000 terms into a full store would fold ten million keys).
    pub(crate) fn add_with(
        &mut self,
        term: NewTerm,
        warn: Option<Reach>,
        at: DateTime<Utc>,
    ) -> Result<AddOutcome, StoreError> {
        let written = validate::term(&term.written)?;
        let agent = term.source == EntrySource::Agent;
        // DD-54: an agent row carries no aliases and matches exactly.
        let (spoken_forms, misrecognitions) = if agent {
            (Vec::new(), Vec::new())
        } else {
            (
                validate::terms(&term.spoken_forms, validate::MAX_SPOKEN_FORMS)?,
                validate::terms(&term.misrecognitions, validate::MAX_MISRECOGNITIONS)?,
            )
        };
        let context_cues = cues(&term.context_cues)?;
        let note = optional(term.note.as_deref(), validate::note)?;
        let reason = optional(term.reason.as_deref(), validate::reason)?;
        let tier = term.tier;
        let person = term.source.is_person();
        let fold = fold_key(&written);
        if let Some(outcome) = self.known_or_tombstoned(&written, &fold, tier, person) {
            return Ok(outcome);
        }
        let Some(status) = self.admitted_status(term.status, tier, person, agent)? else {
            return Ok(AddOutcome::Skipped);
        };
        if person {
            // Only once the row is admitted: an import reports a row refused at the
            // active cap and still writes the rest, so clearing first would lift the
            // person's deletion of a term that was never added.
            self.tombstones
                .retain(|stone| !(stone.tier == tier && stone.fold_key == fold));
        }

        let same_letters = warn.and_then(|reach| {
            self.entries
                .iter()
                .filter(|e| reach.lists(e.tier) && e.written != written && e.fold_key() == fold)
                .map(|e| e.written.clone())
                .min()
        });
        let match_policy = if agent {
            MatchPolicy::Exact
        } else {
            term.match_policy
                .unwrap_or_else(|| MatchPolicy::default_for(term.kind, &written))
        };
        let count = if person { 2 } else { 1 };
        let id = EntryId::random();
        self.entries.push(Entry {
            id: id.clone(),
            rev: 1,
            written,
            tier,
            status,
            source: term.source,
            reviewed: !agent,
            spoken_forms,
            misrecognitions: misrecognitions
                .into_iter()
                .map(|text| Misrecognition {
                    text,
                    count,
                    last_seen: at,
                })
                .collect(),
            kind: term.kind,
            match_policy,
            bias: term.bias,
            context_cues,
            note,
            reason,
            suggested_in: term.suggested_in,
            seen: 1,
            seen_in: Vec::new(),
            created_at: at,
            updated_at: at,
            provenance: None,
            affiliation: if tier.is_private() {
                term.affiliation
            } else {
                None
            },
        });
        Ok(AddOutcome::Created {
            id,
            status,
            same_letters,
        })
    }

    /// What an add finds before it writes: the row already in `tier` (by written form,
    /// or for a writer other than the person by fold key), or a tombstone that stops such
    /// a writer. A person's add passes the tombstone, which [`Self::add_with`] clears once
    /// the row is admitted.
    fn known_or_tombstoned(
        &self,
        written: &str,
        fold: &str,
        tier: EntryTier,
        person: bool,
    ) -> Option<AddOutcome> {
        let exists = |id: &EntryId| AddOutcome::Exists { id: id.clone() };
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| entry.tier == tier && entry.written == written)
        {
            return Some(exists(&existing.id));
        }
        if person {
            return None;
        }
        if self.tombstoned(fold, tier) {
            return Some(AddOutcome::Skipped);
        }
        self.entries
            .iter()
            .find(|entry| entry.tier == tier && entry.fold_key() == fold)
            .map(|existing| exists(&existing.id))
    }

    /// The status a new row gets under the caps of `tier` (DD-41), or `None` when the
    /// suggestions are full and nothing could be evicted. A person's term beyond the
    /// active cap is refused; any other writer's lands suggested beyond the active or the
    /// unreviewed agent cap.
    fn admitted_status(
        &mut self,
        requested: EntryStatus,
        tier: EntryTier,
        person: bool,
        agent: bool,
    ) -> Result<Option<EntryStatus>, StoreError> {
        let mut status = requested;
        if status == EntryStatus::Active {
            let full =
                self.count_in(tier, |e| e.status == EntryStatus::Active) >= MAX_ACTIVE_PER_TIER;
            if person && full {
                return Err(StoreError::Cap(CapKind::Active));
            }
            let unreviewed_full =
                agent && self.count_in(tier, unreviewed_agent) >= MAX_UNREVIEWED_AGENT_PER_TIER;
            if !person && (full || unreviewed_full) {
                status = EntryStatus::Suggested;
            }
        }
        let suggestions_full = status == EntryStatus::Suggested
            && self.count_in(tier, |e| e.status == EntryStatus::Suggested)
                >= MAX_SUGGESTIONS_PER_TIER;
        if suggestions_full && !self.evict_oldest_auto(tier) {
            return Ok(None);
        }
        Ok(Some(status))
    }

    /// The oldest suggested `auto` row of `tier` seen once, removed without a tombstone.
    fn evict_oldest_auto(&mut self, tier: EntryTier) -> bool {
        let oldest = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.tier == tier
                    && e.status == EntryStatus::Suggested
                    && e.source == EntrySource::Auto
                    && e.seen == 1
            })
            .min_by(|(_, a), (_, b)| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)))
            .map(|(i, _)| i);
        match oldest {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    /// Write the person's tombstone for (`fold`, `tier`), dropping the oldest of that
    /// tier beyond 5,000.
    fn tombstone(&mut self, fold: String, tier: EntryTier, at: DateTime<Utc>) {
        if let Some(stone) = self
            .tombstones
            .iter_mut()
            .find(|stone| stone.tier == tier && stone.fold_key == fold)
        {
            stone.at = stone.at.max(at);
            return;
        }
        self.tombstones.push(Tombstone {
            fold_key: fold,
            tier,
            at,
        });
        if self.tombstones.iter().filter(|s| s.tier == tier).count() > MAX_TOMBSTONES_PER_TIER {
            if let Some(oldest) = self
                .tombstones
                .iter()
                .enumerate()
                .filter(|(_, s)| s.tier == tier)
                .min_by(|(_, a), (_, b)| a.at.cmp(&b.at).then(a.fold_key.cmp(&b.fold_key)))
                .map(|(i, _)| i)
            {
                self.tombstones.remove(oldest);
            }
        }
    }

    /// Delete a row and write a tombstone for its key in its tier.
    pub fn remove(&mut self, id: &EntryId, at: DateTime<Utc>) -> Result<Entry, StoreError> {
        let index = self.index_of(id)?;
        let entry = self.entries.remove(index);
        self.tombstone(entry.fold_key(), entry.tier, at);
        Ok(entry)
    }

    /// Accept a suggestion (`active`, `reviewed`), or keep an agent row the person has
    /// not reviewed. Accepting into a tier at its active cap is refused. Accepting a listed
    /// chat suggestion counts as a **Keep** in [`super::ChatLearning`].
    pub fn accept(&mut self, id: &EntryId, at: DateTime<Utc>) -> Result<(), StoreError> {
        let index = self.index_of(id)?;
        let tier = self.entries[index].tier;
        if self.entries[index].status == EntryStatus::Suggested
            && self.count_in(tier, |e| e.status == EntryStatus::Active) >= MAX_ACTIVE_PER_TIER
        {
            return Err(StoreError::Cap(CapKind::Active));
        }
        let entry = &mut self.entries[index];
        if entry.status == EntryStatus::Active && entry.reviewed {
            return Ok(());
        }
        let measured = Self::listed_chat_suggestion(entry);
        entry.status = EntryStatus::Active;
        entry.reviewed = true;
        entry.seen_in.clear();
        entry.touch(at);
        if measured {
            self.record_chat_decision(true, at);
        }
        Ok(())
    }

    /// Change a row's fields. `expected_rev` is the revision the person saw; a row that
    /// changed since answers [`StoreError::Stale`]. A new written form may not repeat
    /// another row's in the same tier ([`StoreError::Duplicate`]). A new spelling or kind
    /// with no `match` of its own moves `match` with the defaults ([`match_after_edit`]).
    pub fn edit(
        &mut self,
        id: &EntryId,
        expected_rev: Option<u64>,
        change: EntryEdit,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let index = self.index_of(id)?;
        let current_rev = self.entries[index].rev;
        if expected_rev.is_some_and(|rev| rev != current_rev) {
            return Err(StoreError::Stale { current_rev });
        }
        let written = change.written.as_deref().map(validate::term).transpose()?;
        let spoken_forms = change
            .spoken_forms
            .as_deref()
            .map(|forms| validate::terms(forms, validate::MAX_SPOKEN_FORMS))
            .transpose()?;
        let context_cues = change.context_cues.as_deref().map(cues).transpose()?;
        let note = change
            .note
            .map(|note| optional(note.as_deref(), validate::note))
            .transpose()?;
        let tier = self.entries[index].tier;
        if let Some(written) = &written {
            if self
                .entries
                .iter()
                .any(|e| e.tier == tier && &e.written == written && &e.id != id)
            {
                return Err(StoreError::Duplicate);
            }
        }

        let entry = &mut self.entries[index];
        let before = entry.clone();
        if let Some(written) = written {
            entry.written = written;
        }
        if let Some(forms) = spoken_forms {
            entry.spoken_forms = forms;
        }
        if let Some(kind) = change.kind {
            entry.kind = kind;
        }
        if let Some(note) = note {
            entry.note = note;
        }
        match change.match_policy {
            Some(policy) => entry.match_policy = policy,
            None if entry.written != before.written || entry.kind != before.kind => {
                entry.match_policy = match_after_edit(&before, entry);
            }
            None => {}
        }
        if let Some(bias) = change.bias {
            entry.bias = bias;
        }
        if let Some(cues) = context_cues {
            entry.context_cues = cues;
        }
        if *entry != before {
            entry.touch(at);
        }
        Ok(())
    }

    /// Union spoken forms and misrecognitions into a row, within its caps; `true` when
    /// the row changed. An added misrecognition counts 2, since the person supplied it.
    pub(crate) fn merge_forms(
        &mut self,
        index: usize,
        spoken_forms: &[String],
        misrecognitions: &[String],
        at: DateTime<Utc>,
    ) -> bool {
        let entry = &mut self.entries[index];
        let mut changed = false;
        for form in spoken_forms {
            if entry.spoken_forms.len() < validate::MAX_SPOKEN_FORMS
                && !entry.spoken_forms.contains(form)
            {
                entry.spoken_forms.push(form.clone());
                changed = true;
            }
        }
        for text in misrecognitions {
            let room = entry.misrecognitions.len() < validate::MAX_MISRECOGNITIONS;
            match entry.misrecognitions.iter_mut().find(|m| &m.text == text) {
                Some(known) if known.count < 2 => {
                    known.count = 2;
                    changed = true;
                }
                Some(_) => {}
                None if room => {
                    entry.misrecognitions.push(Misrecognition {
                        text: text.clone(),
                        count: 2,
                        last_seen: at,
                    });
                    changed = true;
                }
                None => {}
            }
        }
        if changed {
            entry.touch(at);
        }
        changed
    }

    /// Delete the rows a person confirmed from an earlier read, each with its tombstone:
    /// only those still here and still `still` (a suggestion another surface accepted
    /// since is kept), and never a row added since, because the person did not see it.
    /// The rows deleted.
    pub fn remove_confirmed(
        &mut self,
        ids: &[EntryId],
        still: impl Fn(&Entry) -> bool,
        at: DateTime<Utc>,
    ) -> Vec<Entry> {
        let doomed: Vec<EntryId> = ids
            .iter()
            .filter(|id| self.find(id).is_some_and(&still))
            .cloned()
            .collect();
        doomed
            .iter()
            .filter_map(|id| self.remove(id, at).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::now;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + seconds, 0).unwrap()
    }

    fn created(outcome: AddOutcome) -> EntryId {
        match outcome {
            AddOutcome::Created { id, .. } => id,
            other => panic!("expected a new row, got {other:?}"),
        }
    }

    fn by(source: EntrySource, written: &str, tier: EntryTier) -> NewTerm {
        NewTerm {
            source,
            ..NewTerm::person(written, tier)
        }
    }

    /// DD-35: a private add of a term that exists as public creates a second row and
    /// leaves the public row exactly as it was.
    #[test]
    fn a_private_add_of_a_public_term_leaves_the_public_row_untouched() {
        let mut file = StoreFile::default();
        let public = created(
            file.add(
                NewTerm::person("EPAS1", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        let before = serde_json::to_vec(file.find(&public).unwrap()).unwrap();
        let private = NewTerm {
            affiliation: Some(RowAffiliation {
                institutions: vec!["ucsf".into()],
            }),
            ..NewTerm::person("EPAS1", EntryTier::Private)
        };
        let outcome = file.add(private, Reach::All, at(10)).unwrap();
        let AddOutcome::Created { same_letters, .. } = &outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(same_letters, &None, "twins are not a same letters case");
        assert_eq!(
            serde_json::to_vec(file.find(&public).unwrap()).unwrap(),
            before
        );
        assert_eq!(file.entries.len(), 2);
        assert_eq!(
            file.add(
                NewTerm::person("EPAS1", EntryTier::Public),
                Reach::All,
                at(20)
            )
            .unwrap(),
            AddOutcome::Exists { id: public }
        );
    }

    #[test]
    fn a_public_row_never_carries_an_affiliation() {
        let mut file = StoreFile::default();
        let id = created(
            file.add(
                NewTerm {
                    affiliation: Some(RowAffiliation::unknown()),
                    ..NewTerm::person("UCSF", EntryTier::Public)
                },
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        assert_eq!(file.find(&id).unwrap().affiliation, None);
        let private = created(
            file.add(
                NewTerm {
                    affiliation: Some(RowAffiliation::unknown()),
                    ..NewTerm::person("UCSF", EntryTier::Private)
                },
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        assert_eq!(
            file.find(&private).unwrap().affiliation,
            Some(RowAffiliation::unknown())
        );
    }

    /// MSG-SAME-LETTERS: an add whose letters fold like a listed row's is admitted with
    /// the warning, and the warning consults only rows the caller can list.
    #[test]
    fn the_same_letters_warning_counts_only_listed_rows() {
        let mut file = StoreFile::default();
        file.add(
            NewTerm::person("Epas1", EntryTier::Private),
            Reach::All,
            at(0),
        )
        .unwrap();
        let outcome = file
            .add(
                NewTerm::person("EPAS1", EntryTier::Public),
                Reach::Public,
                at(1),
            )
            .unwrap();
        assert!(
            matches!(
                outcome,
                AddOutcome::Created {
                    same_letters: None,
                    ..
                }
            ),
            "a public caller learns nothing of a private row: {outcome:?}"
        );
        let outcome = file
            .add(
                NewTerm::person("E-PAS-1", EntryTier::Public),
                Reach::All,
                at(2),
            )
            .unwrap();
        assert!(
            matches!(&outcome, AddOutcome::Created { same_letters: Some(w), .. } if w == "EPAS1"),
            "{outcome:?}"
        );
        assert_eq!(file.entries.len(), 3, "a warning, never a merge");
    }

    /// Tombstones are per (`fold_key`, `tier`): a deletion stops other writers in that
    /// tier only, and a person's add clears it.
    #[test]
    fn tombstones_per_tier() {
        let mut file = StoreFile::default();
        let id = created(
            file.add(
                NewTerm::person("PHD2", EntryTier::Private),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        file.remove(&id, at(1)).unwrap();
        assert!(file.tombstoned("phd2", EntryTier::Private));
        assert!(!file.tombstoned("phd2", EntryTier::Public));
        for source in [
            EntrySource::Agent,
            EntrySource::Auto,
            EntrySource::Correction,
        ] {
            assert_eq!(
                file.add(by(source, "PhD-2", EntryTier::Private), Reach::All, at(2))
                    .unwrap(),
                AddOutcome::Skipped,
                "{source:?}"
            );
        }
        created(
            file.add(
                by(EntrySource::Agent, "PHD2", EntryTier::Public),
                Reach::All,
                at(3),
            )
            .unwrap(),
        );
        created(
            file.add(
                NewTerm::person("PHD2", EntryTier::Private),
                Reach::All,
                at(4),
            )
            .unwrap(),
        );
        assert!(
            !file.tombstoned("phd2", EntryTier::Private),
            "the person's add clears it"
        );
    }

    #[test]
    fn the_oldest_tombstone_goes_beyond_the_cap() {
        let mut file = StoreFile::default();
        for i in 0..MAX_TOMBSTONES_PER_TIER {
            file.tombstone(format!("t{i}"), EntryTier::Public, at(i as i64));
        }
        file.tombstone("private".into(), EntryTier::Private, at(0));
        file.tombstone("newest".into(), EntryTier::Public, at(99_999));
        assert_eq!(
            file.tombstones
                .iter()
                .filter(|s| s.tier == EntryTier::Public)
                .count(),
            MAX_TOMBSTONES_PER_TIER
        );
        assert!(!file.tombstoned("t0", EntryTier::Public));
        assert!(file.tombstoned("newest", EntryTier::Public));
        assert!(file.tombstoned("private", EntryTier::Private));
    }

    fn fill(
        file: &mut StoreFile,
        tier: EntryTier,
        n: usize,
        status: EntryStatus,
        source: EntrySource,
    ) {
        for i in 0..n {
            file.entries.push(Entry {
                id: EntryId::random(),
                rev: 1,
                written: format!("{}{i}{}", source.as_str(), tier.as_str()),
                tier,
                status,
                source,
                reviewed: source != EntrySource::Agent,
                spoken_forms: vec![],
                misrecognitions: vec![],
                kind: None,
                match_policy: MatchPolicy::Exact,
                bias: BiasPolicy::Auto,
                context_cues: vec![],
                note: None,
                reason: None,
                suggested_in: None,
                seen: 1,
                seen_in: Vec::new(),
                created_at: at(i as i64),
                updated_at: at(i as i64),
                provenance: None,
                affiliation: None,
            });
        }
    }

    /// DD-41: caps count only the tier being written. At 5,000 active public rows a
    /// person's public add is refused, a private add is not, and an agent's term lands
    /// suggested.
    #[test]
    fn caps_per_tier() {
        let mut file = StoreFile::default();
        fill(
            &mut file,
            EntryTier::Public,
            MAX_ACTIVE_PER_TIER,
            EntryStatus::Active,
            EntrySource::User,
        );
        assert!(matches!(
            file.add(
                NewTerm::person("UCSF", EntryTier::Public),
                Reach::All,
                at(0)
            ),
            Err(StoreError::Cap(CapKind::Active))
        ));
        created(
            file.add(
                NewTerm::person("UCSF", EntryTier::Private),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        let outcome = file
            .add(
                by(EntrySource::Agent, "Biorouter", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap();
        assert!(
            matches!(
                outcome,
                AddOutcome::Created {
                    status: EntryStatus::Suggested,
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn unreviewed_agent_rows_and_suggestions_are_capped_per_tier() {
        let mut file = StoreFile::default();
        fill(
            &mut file,
            EntryTier::Private,
            MAX_UNREVIEWED_AGENT_PER_TIER,
            EntryStatus::Active,
            EntrySource::Agent,
        );
        let outcome = file
            .add(
                by(EntrySource::Agent, "EPAS1", EntryTier::Private),
                Reach::All,
                at(0),
            )
            .unwrap();
        assert!(matches!(
            outcome,
            AddOutcome::Created {
                status: EntryStatus::Suggested,
                ..
            }
        ));
        let outcome = file
            .add(
                by(EntrySource::Agent, "EPAS1", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap();
        assert!(
            matches!(
                outcome,
                AddOutcome::Created {
                    status: EntryStatus::Active,
                    ..
                }
            ),
            "the public tier has room: {outcome:?}"
        );

        // Full suggestions: the oldest `auto` row seen once makes room, then nothing does.
        let mut file = StoreFile::default();
        fill(
            &mut file,
            EntryTier::Public,
            MAX_SUGGESTIONS_PER_TIER - 1,
            EntryStatus::Suggested,
            EntrySource::Correction,
        );
        fill(
            &mut file,
            EntryTier::Public,
            1,
            EntryStatus::Suggested,
            EntrySource::Auto,
        );
        let suggest = |w: &str| NewTerm {
            status: EntryStatus::Suggested,
            ..by(EntrySource::Correction, w, EntryTier::Public)
        };
        created(file.add(suggest("NEWONE"), Reach::All, at(0)).unwrap());
        assert_eq!(
            file.count_in(EntryTier::Public, |e| e.status == EntryStatus::Suggested),
            MAX_SUGGESTIONS_PER_TIER
        );
        assert!(
            file.rows_written("auto0public", Reach::All).is_empty(),
            "evicted"
        );
        assert_eq!(
            file.add(suggest("NEWTWO"), Reach::All, at(0)).unwrap(),
            AddOutcome::Skipped
        );
    }

    #[test]
    fn edits_check_the_revision_and_keep_written_forms_unique() {
        let mut file = StoreFile::default();
        let a = created(
            file.add(
                NewTerm::person("EPAS1", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        created(
            file.add(
                NewTerm::person("PHD2", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        let edit = |written: &str| EntryEdit {
            written: Some(written.into()),
            ..EntryEdit::default()
        };
        assert!(matches!(
            file.edit(&a, Some(7), edit("EPAS-1"), at(1)),
            Err(StoreError::Stale { current_rev: 1 })
        ));
        assert!(matches!(
            file.edit(&a, Some(1), edit("PHD2"), at(1)),
            Err(StoreError::Duplicate)
        ));
        assert!(matches!(
            file.edit(&a, Some(1), edit("a\nb"), at(1)),
            Err(StoreError::Invalid(ValidationCode::LineBreak))
        ));
        file.edit(&a, Some(1), edit("EPAS-1"), at(1)).unwrap();
        assert_eq!(file.find(&a).unwrap().rev, 2);
        file.edit(&a, Some(2), edit("EPAS-1"), at(2)).unwrap();
        assert_eq!(
            file.find(&a).unwrap().rev,
            2,
            "an edit that changes nothing keeps rev"
        );
    }

    #[test]
    fn agent_rows_carry_no_aliases_and_wait_for_review() {
        let mut file = StoreFile::default();
        let id = created(
            file.add(
                NewTerm {
                    spoken_forms: vec!["epas one".into()],
                    misrecognitions: vec!["a pass one".into()],
                    match_policy: Some(MatchPolicy::Fuzzy),
                    ..by(EntrySource::Agent, "EPAS1", EntryTier::Public)
                },
                Reach::All,
                now(),
            )
            .unwrap(),
        );
        let row = file.find(&id).unwrap();
        assert!(row.spoken_forms.is_empty() && row.misrecognitions.is_empty());
        assert_eq!(row.match_policy, MatchPolicy::Exact);
        assert!(!row.reviewed);
        file.accept(&id, now()).unwrap();
        assert!(file.find(&id).unwrap().reviewed);
    }

    /// A confirmed delete takes the rows the person saw, each with its tombstone, and
    /// only those still matching: a row added since and a suggestion accepted since stay.
    #[test]
    fn a_confirmed_delete_takes_only_the_rows_still_matching() {
        let mut file = StoreFile::default();
        let epas = created(
            file.add(
                NewTerm::person("EPAS1", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        let suggest = |w: &str| NewTerm {
            status: EntryStatus::Suggested,
            ..by(EntrySource::Auto, w, EntryTier::Public)
        };
        let spoke = created(file.add(suggest("SPOKE"), Reach::All, at(0)).unwrap());
        let vhl = created(file.add(suggest("VHL"), Reach::All, at(0)).unwrap());
        let seen = [epas.clone(), spoke.clone(), vhl.clone()];

        // Meanwhile another surface accepts VHL and adds PHD2.
        file.accept(&vhl, at(1)).unwrap();
        file.add(
            NewTerm::person("PHD2", EntryTier::Public),
            Reach::All,
            at(1),
        )
        .unwrap();

        let suggested = |e: &Entry| e.status == EntryStatus::Suggested;
        let gone = file.remove_confirmed(&seen, suggested, at(2));
        assert_eq!(
            gone.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
            [spoke]
        );
        assert!(file.tombstoned("spoke", EntryTier::Public));
        let gone = file.remove_confirmed(&seen, |_| true, at(3));
        assert_eq!(gone.len(), 2, "EPAS1 and VHL; SPOKE is already gone");
        let left: Vec<&str> = file.entries.iter().map(|e| e.written.as_str()).collect();
        assert_eq!(left, ["PHD2"], "a row added since the read stays");
    }

    /// An edit of the spelling or the kind moves a `match` nobody chose with the
    /// defaults, so a long name edited into a short symbol stops being fuzzy; an explicit
    /// `match` in the edit, a `prompt_only` and an agent row's `exact` are kept.
    #[test]
    fn an_edit_moves_an_unchosen_match_with_the_defaults() {
        let mut file = StoreFile::default();
        let mut add = |term: NewTerm| created(file.add(term, Reach::All, at(0)).unwrap());
        let ventolin = add(NewTerm::person("Ventolinn", EntryTier::Public));
        let chosen = add(NewTerm {
            match_policy: Some(MatchPolicy::Exact),
            ..NewTerm::person("Salbutamol", EntryTier::Public)
        });
        let prompt_only = add(NewTerm {
            match_policy: Some(MatchPolicy::PromptOnly),
            ..NewTerm::person("Albuterol", EntryTier::Public)
        });
        let agent = add(by(EntrySource::Agent, "Levalbuterol", EntryTier::Public));
        let kind_change = add(NewTerm::person("Formoterol", EntryTier::Public));
        let explicit = add(NewTerm::person("Budesonide", EntryTier::Public));
        let policy = |file: &StoreFile, id: &EntryId| file.find(id).unwrap().match_policy;
        assert_eq!(policy(&file, &ventolin), MatchPolicy::Fuzzy);

        let respell = |written: &str| EntryEdit {
            written: Some(written.into()),
            ..EntryEdit::default()
        };
        file.edit(&ventolin, None, respell("HRAS"), at(1)).unwrap();
        assert_eq!(policy(&file, &ventolin), MatchPolicy::Exact, "HRAS");
        file.edit(&ventolin, None, respell("Ventolin"), at(2))
            .unwrap();
        assert_eq!(
            policy(&file, &ventolin),
            MatchPolicy::Fuzzy,
            "the default again"
        );

        file.edit(&chosen, None, respell("Salbutamols"), at(1))
            .unwrap();
        assert_eq!(
            policy(&file, &chosen),
            MatchPolicy::Exact,
            "a chosen exact stays"
        );
        file.edit(&prompt_only, None, respell("ARNT"), at(1))
            .unwrap();
        assert_eq!(policy(&file, &prompt_only), MatchPolicy::PromptOnly);
        file.edit(&agent, None, respell("Levalbuterols"), at(1))
            .unwrap();
        assert_eq!(policy(&file, &agent), MatchPolicy::Exact, "DD-54");

        let gene = EntryEdit {
            kind: Some(Some(TermKind::GeneSymbol)),
            ..EntryEdit::default()
        };
        file.edit(&kind_change, None, gene, at(1)).unwrap();
        assert_eq!(policy(&file, &kind_change), MatchPolicy::Exact);

        let fuzzy_nras = EntryEdit {
            match_policy: Some(MatchPolicy::Fuzzy),
            ..respell("NRAS")
        };
        file.edit(&explicit, None, fuzzy_nras, at(1)).unwrap();
        assert_eq!(
            policy(&file, &explicit),
            MatchPolicy::Fuzzy,
            "the edit's own match"
        );

        // NRAS's fuzzy is not its default, yet KRAS, a short symbol, never takes fuzzy.
        file.edit(&explicit, None, respell("KRAS"), at(2)).unwrap();
        assert_eq!(policy(&file, &explicit), MatchPolicy::Exact);
    }

    /// A person's add refused at the active cap leaves that tier's tombstone, so the
    /// writers it stops are still stopped.
    #[test]
    fn an_add_refused_at_the_cap_keeps_the_tombstone() {
        let mut file = StoreFile::default();
        let id = created(
            file.add(
                NewTerm::person("SPOKE", EntryTier::Public),
                Reach::All,
                at(0),
            )
            .unwrap(),
        );
        file.remove(&id, at(1)).unwrap();
        fill(
            &mut file,
            EntryTier::Public,
            MAX_ACTIVE_PER_TIER,
            EntryStatus::Active,
            EntrySource::User,
        );
        assert!(matches!(
            file.add(
                NewTerm::person("SPOKE", EntryTier::Public),
                Reach::All,
                at(2)
            ),
            Err(StoreError::Cap(CapKind::Active))
        ));
        assert!(file.tombstoned("spoke", EntryTier::Public));
    }
    #[test]
    fn auto_suggestions_count_distinct_sessions_and_never_demote() {
        let mut file = StoreFile::default();
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "a", None, at(0))
            .unwrap();
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "a", None, at(1))
            .unwrap();
        assert_eq!(file.entries[0].seen, 1);
        assert_eq!(file.entries[0].status, EntryStatus::Suggested);
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "b", None, at(2))
            .unwrap();
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "a", None, at(3))
            .unwrap();
        assert_eq!(file.entries[0].seen, 2);
        let rev = file.entries[0].rev;
        file.accept(&file.entries[0].id.clone(), at(4)).unwrap();
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "c", None, at(5))
            .unwrap();
        assert_eq!(file.entries[0].status, EntryStatus::Active);
        assert_eq!(file.entries[0].rev, rev + 1);
    }

    #[test]
    fn a_private_auto_row_keeps_its_creation_institutions() {
        let mut file = StoreFile::default();
        file.observe_auto_suggestion(
            "EPAS1",
            EntryTier::Private,
            "first",
            Some(RowAffiliation {
                institutions: vec!["ucsf".into()],
            }),
            at(0),
        )
        .unwrap();
        file.observe_auto_suggestion(
            "EPAS1",
            EntryTier::Private,
            "second",
            Some(RowAffiliation {
                institutions: vec!["stanford".into()],
            }),
            at(1),
        )
        .unwrap();
        assert_eq!(file.entries[0].seen, 2);
        assert_eq!(
            file.entries[0].affiliation.as_ref().unwrap().institutions,
            ["ucsf"]
        );
    }

    #[test]
    fn auto_suggestions_respect_tier_and_tombstones() {
        let mut file = StoreFile::default();
        file.observe_auto_suggestion("UCSF", EntryTier::Public, "a", None, at(0))
            .unwrap();
        let public_id = file.entries[0].id.clone();
        file.observe_auto_suggestion("UCSF", EntryTier::Private, "a", None, at(1))
            .unwrap();
        assert_eq!(file.entries.len(), 2);
        assert_eq!(file.entries[0].id, public_id);
        let private_id = file.entries[1].id.clone();
        file.remove(&private_id, at(2)).unwrap();
        file.observe_auto_suggestion("UCSF", EntryTier::Private, "b", None, at(3))
            .unwrap();
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].id, public_id);
    }

    /// DD-57 skips rows the caller can already list. A private caller lists every public
    /// row but an `auto` row seen once, so those get no private twin.
    #[test]
    fn a_private_sighting_of_a_term_the_caller_can_list_adds_no_twin() {
        let mut file = StoreFile::default();
        assert!(matches!(
            file.add(
                NewTerm::person("UCSF", EntryTier::Public),
                Reach::All,
                at(0)
            )
            .unwrap(),
            AddOutcome::Created { .. }
        ));
        file.observe_auto_suggestion("UCSF", EntryTier::Private, "a", None, at(1))
            .unwrap();
        assert_eq!(file.entries.len(), 1);
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "a", None, at(2))
            .unwrap();
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "b", None, at(3))
            .unwrap();
        file.observe_auto_suggestion("EPAS1", EntryTier::Private, "c", None, at(4))
            .unwrap();
        assert_eq!(file.entries.len(), 2);
        assert!(file.entries.iter().all(|row| row.tier == EntryTier::Public));
    }

    /// Owner, 2026-11-13: a word counts once per chat, and the chats it needs before it
    /// is listed follow the person's own decisions.
    fn listed_after(file: &mut StoreFile, word: &str, chats: &[&str]) -> bool {
        for (i, chat) in chats.iter().enumerate() {
            file.observe_auto_suggestion(word, EntryTier::Public, chat, None, at(i as i64))
                .unwrap();
        }
        file.entries
            .iter()
            .find(|row| row.written == word)
            .is_some_and(|row| row.seen >= 2)
    }

    /// `kept` listed suggestions kept and `dismissed` dismissed, through the operations the
    /// review surfaces call.
    fn decide(file: &mut StoreFile, kept: usize, dismissed: usize) {
        let first = file.chat_learning.decisions() as usize;
        for i in 0..kept + dismissed {
            let word = format!("DEC{}X", first + i);
            assert!(listed_after(file, &word, &["one", "two", "three", "four"]));
            let id = file.entries.iter().find(|r| r.written == word).unwrap().id.clone();
            if i < kept {
                file.accept(&id, at(100)).unwrap();
            } else {
                assert_eq!(file.dismiss_suggestions(&[id], at(100)).len(), 1);
            }
        }
    }

    #[test]
    fn a_word_counts_once_per_chat_and_is_listed_at_two_chats() {
        let mut file = StoreFile::default();
        assert!(!listed_after(&mut file, "EPAS1", &["a", "a", "a"]));
        let row = &file.entries[0];
        assert_eq!((row.seen, row.seen_in.len()), (1, 1));
        assert!(!row.seen_in.iter().any(|d| d == "a"), "a digest, not the chat id");
        assert!(listed_after(&mut file, "EPAS1", &["b"]));
        assert_eq!(file.entries[0].seen_in.len(), 2);
        assert_eq!(file.entries[0].suggested_in.as_deref(), Some("b"));
    }

    #[test]
    fn keep_and_dismiss_of_listed_chat_suggestions_are_counted() {
        let mut file = StoreFile::default();
        decide(&mut file, 2, 3);
        assert_eq!(file.chat_learning.kept, 2);
        assert_eq!(file.chat_learning.dismissed, 3);
        assert_eq!(file.chat_learning.recent, vec![true, true, false, false, false]);
        // Neither an unlisted suggestion, nor an agent's or a person's row, measures the
        // chat extractor.
        file.observe_auto_suggestion("HIDDEN1", EntryTier::Public, "x", None, at(1))
            .unwrap();
        let hidden = file.entries.iter().find(|r| r.written == "HIDDEN1").unwrap().id.clone();
        assert_eq!(file.dismiss_suggestions(&[hidden], at(2)).len(), 1);
        let agent = created(
            file.add(by(EntrySource::Agent, "AGENT1", EntryTier::Public), Reach::All, at(3))
                .unwrap(),
        );
        file.accept(&agent, at(4)).unwrap();
        assert_eq!(file.chat_learning.decisions(), 5);
    }

    #[test]
    fn a_dismissed_word_is_never_suggested_again() {
        let mut file = StoreFile::default();
        assert!(listed_after(&mut file, "EPAS1", &["a", "b"]));
        let id = file.entries[0].id.clone();
        file.dismiss_suggestions(&[id], at(5));
        assert!(file.entries.is_empty());
        assert!(!listed_after(&mut file, "EPAS1", &["c", "d", "e", "f"]));
        assert!(file.entries.is_empty(), "the tombstone stops it");
    }

    #[test]
    fn dismissing_only_takes_rows_that_are_still_suggestions() {
        let mut file = StoreFile::default();
        assert!(listed_after(&mut file, "EPAS1", &["a", "b"]));
        let id = file.entries[0].id.clone();
        file.accept(&id, at(5)).unwrap();
        assert!(file.dismiss_suggestions(&[id], at(6)).is_empty());
        assert_eq!(file.entries[0].status, EntryStatus::Active);
        assert_eq!(file.chat_learning.recent, vec![true]);
    }

    #[test]
    fn most_dismissed_means_a_word_needs_more_chats() {
        let mut file = StoreFile::default();
        decide(&mut file, 9, 11);
        assert_eq!(file.chat_learning.required_chats(), 3);
        assert!(!listed_after(&mut file, "EPAS1", &["a", "b"]));
        assert!(listed_after(&mut file, "EPAS1", &["c"]));
        decide(&mut file, 0, 20);
        assert_eq!(file.chat_learning.required_chats(), 4, "9 kept of 40");
        assert!(!listed_after(&mut file, "HIF2A", &["a", "b", "c"]));
        assert!(listed_after(&mut file, "HIF2A", &["d"]));
        assert_eq!(file.entries.iter().find(|r| r.written == "HIF2A").unwrap().seen_in.len(), 4);
    }

    #[test]
    fn keeping_more_lowers_the_threshold_and_lists_what_now_qualifies() {
        let mut file = StoreFile::default();
        decide(&mut file, 9, 11);
        assert!(!listed_after(&mut file, "EPAS1", &["a", "b"]));
        decide(&mut file, 2, 0);
        assert_eq!(file.chat_learning.required_chats(), 2);
        let row = file.entries.iter().find(|r| r.written == "EPAS1").unwrap();
        assert_eq!(row.seen, 2, "a suggestion already found in two chats is listed now");
    }

    #[test]
    fn a_suggestion_from_before_chats_were_counted_counts_its_chat() {
        let mut file = StoreFile::default();
        file.observe_auto_suggestion("EPAS1", EntryTier::Public, "a", None, at(0))
            .unwrap();
        file.entries[0].seen_in.clear();
        assert!(!listed_after(&mut file, "EPAS1", &["a"]), "its own chat again");
        assert!(listed_after(&mut file, "EPAS1", &["b"]));
    }

    #[test]
    fn a_store_whose_owner_decided_nothing_writes_no_learning_and_reads_it_back() {
        let mut file = StoreFile::default();
        let before = file.to_bytes();
        assert!(!String::from_utf8_lossy(&before).contains("chat_learning"));
        decide(&mut file, 1, 1);
        let bytes = file.to_bytes();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(r#""chat_learning": {"recent":[true,false],"kept":1,"dismissed":1}"#));
        assert_eq!(StoreFile::parse(&bytes).unwrap(), file);
    }
}
