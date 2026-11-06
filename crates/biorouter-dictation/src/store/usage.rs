//! `usage.json` ("Usage"): how often dictation wrote each row and how often the person
//! corrected text to it, in two maps of one shape. Every bump goes to `all`; a
//! dictation decoded with the public rows table, and a correction stamped public, also
//! bump `public`, so ranking for a public scope request never changes with private
//! dictations (oracle rule 11).
//!
//! A separate file, so a usage bump never changes an entry's `rev` (a spurious `412`
//! on the person's open edit) and never rewrites the entries file per utterance
//! (DD-33). The service batches bumps and flushes them under the store's lock with a
//! short budget; a lost bump is acceptable.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::schema::EntryId;

/// One row's counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageRecord {
    pub uses: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub corrected_to: u64,
}

/// `usage.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageFile {
    #[serde(default)]
    pub all: BTreeMap<String, UsageRecord>,
    #[serde(default)]
    pub public: BTreeMap<String, UsageRecord>,
}

/// Which maps a bump reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageScope {
    /// A private scope dictation or a private correction: `all` only.
    All,
    /// A public scope dictation or a public correction: `all` and `public`.
    Public,
}

/// What one flush adds to a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageDelta {
    pub uses: u64,
    pub corrected_to: u64,
    /// The latest use in the batch, if it held one.
    pub last_used_at: Option<DateTime<Utc>>,
}

impl UsageRecord {
    fn add(&mut self, delta: &UsageDelta) {
        self.uses = self.uses.saturating_add(delta.uses);
        self.corrected_to = self.corrected_to.saturating_add(delta.corrected_to);
        if let Some(at) = delta.last_used_at {
            if self.last_used_at.is_none_or(|before| at > before) {
                self.last_used_at = Some(at);
            }
        }
    }
}

impl UsageFile {
    /// `None` when the bytes are not a usage file this version reads.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).expect("usage serializes");
        bytes.push(b'\n');
        bytes
    }

    /// Apply a batch of bumps.
    pub fn apply(&mut self, deltas: &[(EntryId, UsageScope, UsageDelta)]) {
        for (id, scope, delta) in deltas {
            self.all
                .entry(id.as_str().to_string())
                .or_default()
                .add(delta);
            if *scope == UsageScope::Public {
                self.public
                    .entry(id.as_str().to_string())
                    .or_default()
                    .add(delta);
            }
        }
    }

    /// Drop the counters of rows that no longer exist.
    pub fn retain_ids(&mut self, keep: impl Fn(&str) -> bool) {
        self.all.retain(|id, _| keep(id));
        self.public.retain(|id, _| keep(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_public_bump_reaches_both_maps_and_a_private_one_only_all() {
        let id = EntryId::random();
        let at: DateTime<Utc> = "2026-11-04T17:02:11Z".parse().unwrap();
        let mut usage = UsageFile::default();
        usage.apply(&[
            (
                id.clone(),
                UsageScope::Public,
                UsageDelta {
                    uses: 2,
                    corrected_to: 0,
                    last_used_at: Some(at),
                },
            ),
            (
                id.clone(),
                UsageScope::All,
                UsageDelta {
                    uses: 3,
                    corrected_to: 1,
                    last_used_at: None,
                },
            ),
        ]);
        let all = &usage.all[id.as_str()];
        assert_eq!(
            (all.uses, all.corrected_to, all.last_used_at),
            (5, 1, Some(at))
        );
        let public = &usage.public[id.as_str()];
        assert_eq!((public.uses, public.corrected_to), (2, 0));
        assert_eq!(UsageFile::parse(&usage.to_bytes()), Some(usage));
    }
}
