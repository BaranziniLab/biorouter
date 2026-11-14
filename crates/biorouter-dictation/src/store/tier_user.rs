//! Tier changes, the person's act only (DD-39, P-13).
//!
//! Every other write keeps a row in the tier it was stamped with; an edit or a
//! correction that a higher stamp reaches lands in a private twin instead. The only way
//! to move a row between tiers is [`StoreFile::set_tier`], whose signature takes the
//! proof below, the pattern of the knowledge base tier proof in
//! `biorouter-mcp/src/knowledge/tier_user.rs`.
//!
//! ⚠ **There is no tool for this, and there must never be one.** The proof is built in
//! exactly two files, after the person acted: the CLI's `publicize` and `privatize`,
//! after the typed confirmation at an interactive terminal with `BIOROUTER_TERMINAL`
//! unset, and from PR-16 the tier route, after the user action proof and before the id
//! is read. `tests::the_tier_proof_is_constructed_in_exactly_two_files` walks the tree.

use chrono::{DateTime, Utc};

use super::schema::{
    Entry, EntryId, EntryStatus, EntryTier, Provenance, ProvenanceReason, StoreFile,
};
use super::validate;
use super::{CapKind, StoreError};

/// Proof that a person asked for a tier change. A zero sized type with a private field,
/// so the struct literal is unavailable outside this module and the named constructor
/// is the only door; what caps its callers at two files is the repository walk below,
/// because the callers live in other crates and `pub(in path)` does not cross a crate.
pub struct UserDictionaryTierChange(());

impl UserDictionaryTierChange {
    /// Call only after the person confirmed: the typed phrase at an interactive terminal,
    /// or the user action proof on a route.
    pub fn from_user_action() -> Self {
        Self(())
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self(())
    }
}

/// Union `from`'s spoken forms and misrecognitions into `into`, within the caps.
fn merge_rows(into: &mut Entry, from: &Entry) {
    for form in &from.spoken_forms {
        if into.spoken_forms.len() < validate::MAX_SPOKEN_FORMS && !into.spoken_forms.contains(form)
        {
            into.spoken_forms.push(form.clone());
        }
    }
    for heard in &from.misrecognitions {
        let room = into.misrecognitions.len() < validate::MAX_MISRECOGNITIONS;
        match into
            .misrecognitions
            .iter_mut()
            .find(|m| m.text == heard.text)
        {
            Some(known) => {
                known.count = known.count.max(heard.count);
                known.last_seen = known.last_seen.max(heard.last_seen);
            }
            None if room => {
                into.misrecognitions.push(heard.clone());
            }
            None => {}
        }
    }
    if from.status == EntryStatus::Active {
        into.status = EntryStatus::Active;
        into.reviewed = true;
    }
}

impl StoreFile {
    /// Move a row to `tier`, or merge it into its twin there; the id of the row that
    /// holds the term afterwards.
    ///
    /// Publicize merges a private row into the public twin (union of spoken forms and
    /// misrecognitions, the public note kept, the private row removed) or moves it when
    /// no twin exists, and a public row carries no affiliation. Privatize does the
    /// reverse; a moved row's affiliation is `null` (the person's act with no chat, O-36)
    /// and a twin keeps its own. Either clears a tombstone for the term in `tier` and
    /// records the person's change in `provenance`. A move into a tier at its active cap
    /// is refused.
    pub fn set_tier(
        &mut self,
        id: &EntryId,
        tier: EntryTier,
        _proof: &UserDictionaryTierChange,
        at: DateTime<Utc>,
    ) -> Result<EntryId, StoreError> {
        let index = self
            .entries
            .iter()
            .position(|entry| &entry.id == id)
            .ok_or(StoreError::NotFound)?;
        if self.entries[index].tier == tier {
            return Ok(id.clone());
        }
        let provenance = Some(Provenance {
            reason: if tier.is_private() {
                ProvenanceReason::PrivatizedByUser
            } else {
                ProvenanceReason::PublicizedByUser
            },
            at,
        });
        let written = self.entries[index].written.clone();
        let fold = self.entries[index].fold_key();

        if let Some(twin) = self
            .entries
            .iter()
            .position(|e| e.tier == tier && e.written == written)
        {
            let moved = self.entries.remove(index);
            let twin = if twin > index { twin - 1 } else { twin };
            let target = &mut self.entries[twin];
            merge_rows(target, &moved);
            target.provenance = provenance;
            target.touch(at);
            let kept = target.id.clone();
            self.tombstones
                .retain(|stone| !(stone.tier == tier && stone.fold_key == fold));
            return Ok(kept);
        }

        if self.entries[index].status == EntryStatus::Active
            && self.count_in(tier, |e| e.status == EntryStatus::Active)
                >= super::ops::MAX_ACTIVE_PER_TIER
        {
            return Err(StoreError::Cap(CapKind::Active));
        }
        let entry = &mut self.entries[index];
        entry.tier = tier;
        entry.affiliation = None;
        entry.provenance = provenance;
        entry.touch(at);
        self.tombstones
            .retain(|stone| !(stone.tier == tier && stone.fold_key == fold));
        Ok(id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ops::{AddOutcome, NewTerm, Reach};
    use crate::store::schema::{now, RowAffiliation};
    use std::path::{Path, PathBuf};

    fn add(file: &mut StoreFile, term: NewTerm) -> EntryId {
        match file.add(term, Reach::All, now()).unwrap() {
            AddOutcome::Created { id, .. } => id,
            other => panic!("{other:?}"),
        }
    }

    fn private_with(written: &str, ids: &[&str]) -> NewTerm {
        NewTerm {
            spoken_forms: vec!["epas one".into()],
            affiliation: Some(RowAffiliation {
                institutions: ids.iter().map(|s| s.to_string()).collect(),
            }),
            ..NewTerm::person(written, EntryTier::Private)
        }
    }

    #[test]
    fn publicize_moves_a_row_and_drops_its_affiliation() {
        let mut file = StoreFile::default();
        let id = add(&mut file, private_with("EPAS1", &["ucsf"]));
        let proof = UserDictionaryTierChange::for_test();
        let kept = file
            .set_tier(&id, EntryTier::Public, &proof, now())
            .unwrap();
        assert_eq!(kept, id);
        let row = file.find(&id).unwrap();
        assert_eq!((row.tier, &row.affiliation), (EntryTier::Public, &None));
        assert_eq!(
            row.provenance.as_ref().map(|p| p.reason),
            Some(ProvenanceReason::PublicizedByUser)
        );
        assert_eq!(row.rev, 2);
    }

    #[test]
    fn publicize_merges_into_the_public_twin() {
        let mut file = StoreFile::default();
        let public = add(
            &mut file,
            NewTerm {
                note: Some("the public note".into()),
                spoken_forms: vec!["e pass one".into()],
                ..NewTerm::person("EPAS1", EntryTier::Public)
            },
        );
        let private = add(&mut file, private_with("EPAS1", &["ucsf"]));
        let proof = UserDictionaryTierChange::for_test();
        let kept = file
            .set_tier(&private, EntryTier::Public, &proof, now())
            .unwrap();
        assert_eq!(kept, public);
        assert!(file.find(&private).is_none(), "the private row is removed");
        let row = file.find(&public).unwrap();
        assert_eq!(row.spoken_forms, vec!["e pass one", "epas one"]);
        assert_eq!(row.note.as_deref(), Some("the public note"));
        assert_eq!(row.affiliation, None);
    }

    #[test]
    fn privatize_writes_null_and_a_twin_keeps_its_own() {
        let mut file = StoreFile::default();
        let id = add(&mut file, NewTerm::person("PHD2", EntryTier::Public));
        let proof = UserDictionaryTierChange::for_test();
        file.set_tier(&id, EntryTier::Private, &proof, now())
            .unwrap();
        let row = file.find(&id).unwrap();
        assert_eq!((row.tier, &row.affiliation), (EntryTier::Private, &None));

        let twin = add(&mut file, private_with("EPAS1", &["ucsf"]));
        let public = add(&mut file, NewTerm::person("EPAS1", EntryTier::Public));
        let kept = file
            .set_tier(&public, EntryTier::Private, &proof, now())
            .unwrap();
        assert_eq!(kept, twin);
        assert_eq!(
            file.find(&twin).unwrap().affiliation,
            Some(RowAffiliation {
                institutions: vec!["ucsf".into()]
            })
        );
    }

    /// Every `.rs` file under `crates/`, as (path relative to the workspace, source).
    fn workspace_sources() -> Vec<(String, String)> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        walk(&crates, &mut files);
        files
            .into_iter()
            .filter_map(|path| {
                let source = std::fs::read_to_string(&path).ok()?;
                let relative = path
                    .strip_prefix(&crates)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                Some((format!("crates/{relative}"), source))
            })
            .collect()
    }

    /// P-13: the proof is built in two files at most, the CLI's dictionary commands and
    /// PR-16's route, and the CLI builds it once.
    #[test]
    fn the_tier_proof_is_constructed_in_exactly_two_files() {
        const CLI: &str = "crates/biorouter-cli/src/commands/dictionary.rs";
        const ROUTE: &str = "crates/biorouter-server/src/routes/dictionary.rs";
        let needle = concat!("UserDictionaryTierChange::", "from_user_action(");
        let sources = workspace_sources();
        assert!(
            sources.len() > 400,
            "the walk found only {} files",
            sources.len()
        );
        let mut found: Vec<(String, usize)> = Vec::new();
        for (path, source) in &sources {
            if path.ends_with("biorouter-dictation/src/store/tier_user.rs") {
                continue;
            }
            let calls = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .map(|line| line.matches(needle).count())
                .sum::<usize>();
            if calls > 0 {
                found.push((path.clone(), calls));
            }
        }
        assert!(
            found.iter().all(|(path, _)| path == CLI || path == ROUTE),
            "the tier proof is built outside its two files: {found:?}"
        );
        assert_eq!(
            found.iter().find(|(path, _)| path == CLI).map(|(_, n)| *n),
            Some(1),
            "the CLI builds the proof once, after the typed confirmation: {found:?}"
        );
    }
}
