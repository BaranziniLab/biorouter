//! The dictation history in the host process (DD-96): where it lives, the one handle a
//! process shares, the tier and institutions a new entry is stamped with, and the one
//! predicate that decides whether the agent may read an entry.
//!
//! The store itself is the leaf crate's ([`store`]), which decides nothing about privacy.
//! This module decides two things, each in one function:
//!
//! * [`history_stamp`], the tier and institutions a finished dictation is recorded with.
//!   Private unless the person turned on public by default and nothing about the
//!   dictation says private (6.3 of the history design). Called by the daemon's stream
//!   route and the CLI's recorder, and by nothing else.
//! * [`agent_reads_dictation`], whether one entry may reach the model of a tool call: the
//!   Gate D rule on the call's admitted capability, then P-16's institution rule, asked of
//!   the same capability. It reads only the capability it is handed, never the master
//!   switch or the provider slot, so a call admitted Public stays Public whatever happens
//!   to the chat while it runs.
//!
//! The history never feeds recognition (P-18): nothing in `bias`, the extractor, usage or
//! observations opens it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use biorouter_dictation::recognize::BiasScope;
use biorouter_dictation::store::schema::{normalize_institution_id, MAX_ROW_INSTITUTIONS};

use super::target::ChatReach;
use crate::config::paths::Paths;
use crate::privacy::{
    affiliation, CallCapability, ExtensionClassification, ProviderTier, SessionClassification,
};

/// The leaf crate's history store.
pub use biorouter_dictation::history as store;
pub use biorouter_dictation::history::{
    EntryTier, HistoryCursor, HistoryEntry, HistoryError, HistoryPage, HistorySettings,
    HistoryStore, SettingsChange, Surface, TimeWindow,
};

/// The history's folder under the local data folder.
pub const HISTORY_FOLDER: &str = "dictation/history";

/// The `dictionary` capability's tool that searches the history (DD-100). Managed
/// `dictation.history: false` denies it.
pub const SEARCH_DICTATIONS_TOOL: &str = "dictionary__search_dictations";

/// The extension name a refused institution check names.
const HISTORY_EXTENSION_LABEL: &str = "dictation history";

/// `<local data>/dictation/history`: `~/.local/share/biorouter/dictation/history` on macOS
/// and Linux, `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\history` on Windows (never
/// roamed, so spoken text is not copied between machines), and
/// `$BIOROUTER_PATH_ROOT/data/dictation/history` under a sandbox root.
pub fn history_dir() -> PathBuf {
    Paths::in_local_data_dir(HISTORY_FOLDER)
}

/// The one handle this process shares, so its reads share one cache. A process whose
/// folder moved (a test's sandbox root) gets a handle on the new folder.
pub fn open_history() -> Arc<HistoryStore> {
    static SHARED: Mutex<Option<Arc<HistoryStore>>> = Mutex::new(None);
    let dir = history_dir();
    let mut shared = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
    match shared.as_ref() {
        Some(store) if store.dir() == dir => Arc::clone(store),
        _ => {
            let store = Arc::new(HistoryStore::open(dir));
            *shared = Some(Arc::clone(&store));
            store
        }
    }
}

/// Where a finished dictation went, as far as its tier is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryTarget {
    /// A chat the caller could reach, with its institutions in the dictionary row's
    /// meaning (P-16): `None` for none, `Some([])` when they could not be read.
    Chat {
        reach: ChatReach,
        institutions: Option<Vec<String>>,
    },
    /// A chat id the caller could not reach, or whose row could not be read.
    Unreachable,
    /// A new chat, with the tier its bind would produce.
    NewChat(ProviderTier),
    /// Crew, the clipboard, another app, an app preview or the terminal.
    Outside,
}

/// What a new entry is recorded with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryStamp {
    pub tier: EntryTier,
    /// Private entries only; always `None` on a public one.
    pub institutions: Option<Vec<String>>,
}

/// The tier and institutions a finished dictation is recorded with.
///
/// The tier is private unless `public_by_default` is on and the dictation used no private
/// term (`scope` public); then it is public for a public chat, a new chat whose bind is
/// public, and a dictation outside any chat, and private for a private chat, a chat that
/// could not be reached or read, and a new chat whose bind is private. Unknown fails
/// toward private.
///
/// A private entry's institutions are the union of the target chat's and those of every
/// private dictionary row the stream's bias set used (`private_rows`, each in the row's
/// own meaning): `Some([])`, a model on this computer only, when any of them could not
/// be read, the chat could not be reached, or the union names more than eight; `None`
/// when the union is empty. A target outside any chat, or a new chat, adds none.
pub fn history_stamp(
    public_by_default: bool,
    scope: BiasScope,
    target: &HistoryTarget,
    private_rows: &[Option<Vec<String>>],
) -> HistoryStamp {
    match history_tier(public_by_default, scope, target) {
        EntryTier::Public => HistoryStamp {
            tier: EntryTier::Public,
            institutions: None,
        },
        EntryTier::Private => HistoryStamp {
            tier: EntryTier::Private,
            institutions: private_institutions(target, private_rows),
        },
    }
}

fn history_tier(public_by_default: bool, scope: BiasScope, target: &HistoryTarget) -> EntryTier {
    if !public_by_default || scope == BiasScope::Private {
        return EntryTier::Private;
    }
    match target {
        HistoryTarget::Chat {
            reach: ChatReach::Public,
            ..
        } => EntryTier::Public,
        HistoryTarget::Chat { .. } | HistoryTarget::Unreachable => EntryTier::Private,
        HistoryTarget::NewChat(tier) if tier.is_private() => EntryTier::Private,
        HistoryTarget::NewChat(_) | HistoryTarget::Outside => EntryTier::Public,
    }
}

/// The institutions behind a private entry. See [`history_stamp`].
fn private_institutions(
    target: &HistoryTarget,
    private_rows: &[Option<Vec<String>>],
) -> Option<Vec<String>> {
    let mut owners = BTreeSet::new();
    let mut unknown = false;
    let mut add = |stored: Option<&[String]>| match stored {
        None => {}
        Some([]) => unknown = true,
        Some(ids) => {
            for id in ids {
                match normalize_institution_id(id) {
                    Some(id) => {
                        owners.insert(id);
                    }
                    None => unknown = true,
                }
            }
        }
    };
    match target {
        HistoryTarget::Chat { institutions, .. } => add(institutions.as_deref()),
        HistoryTarget::Unreachable => add(Some(&[])),
        HistoryTarget::NewChat(_) | HistoryTarget::Outside => {}
    }
    for row in private_rows {
        add(row.as_deref());
    }
    if unknown || owners.len() > MAX_ROW_INSTITUTIONS {
        return Some(Vec::new());
    }
    (!owners.is_empty()).then(|| owners.into_iter().collect())
}

fn classification_of(tier: EntryTier) -> SessionClassification {
    match tier {
        EntryTier::Public => SessionClassification::Public,
        EntryTier::Private => SessionClassification::Private,
    }
}

/// Whether `entry` may reach the model of the tool call admitted with `cap`.
///
/// Gate D's rule, on the admitted sample: hidden when the sample enforced the tiers and
/// its tier may not see the entry's. Then each institution the entry stores is asked
/// through the capability's own DR-26 check, one allowlist per institution, exactly as a
/// private dictionary row is asked (P-16): `null` reaches every private model, `[]` only
/// a model on this computer, and a list only a model every listed institution admits.
/// The sample's master switch and mixing policy apply unchanged.
pub fn agent_reads_dictation(entry: &HistoryEntry, cap: CallCapability) -> bool {
    if cap.enforced() && !crate::privacy::visible_to(cap.tier(), classification_of(entry.tier)) {
        return false;
    }
    affiliation::dictionary_row_allowlists(entry.institutions.as_deref())
        .into_iter()
        .all(|allowlist| {
            cap.cross_affiliation_warning(
                HISTORY_EXTENSION_LABEL,
                &ExtensionClassification {
                    tier: ProviderTier::Private,
                    affiliation: allowlist,
                },
            )
            .is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
    use crate::privacy::mixing::{pin_for_test, MixingPolicy};
    use chrono::DateTime;

    fn ids(list: &[&str]) -> Option<Vec<String>> {
        Some(list.iter().map(|s| s.to_string()).collect())
    }

    fn chat(reach: ChatReach, institutions: Option<Vec<String>>) -> HistoryTarget {
        HistoryTarget::Chat {
            reach,
            institutions,
        }
    }

    /// 6.3 of the history design: every setting, scope and target.
    #[test]
    fn the_tier_table() {
        use BiasScope::{Private as PrivateScope, Public as PublicScope};
        use EntryTier::{Private, Public};
        let targets = [
            ("public chat", chat(ChatReach::Public, None), Public),
            ("private chat", chat(ChatReach::Private, None), Private),
            ("unreadable chat", chat(ChatReach::Unknown, None), Private),
            ("unreachable", HistoryTarget::Unreachable, Private),
            (
                "public new chat",
                HistoryTarget::NewChat(ProviderTier::Public),
                Public,
            ),
            (
                "private new chat",
                HistoryTarget::NewChat(ProviderTier::Private),
                Private,
            ),
            ("outside", HistoryTarget::Outside, Public),
        ];
        for (label, target, with_setting) in targets {
            for scope in [PublicScope, PrivateScope] {
                assert_eq!(
                    history_stamp(false, scope, &target, &[]).tier,
                    Private,
                    "{label}: off is always private"
                );
            }
            assert_eq!(
                history_stamp(true, PrivateScope, &target, &[]).tier,
                Private,
                "{label}: private terms stay private"
            );
            let stamp = history_stamp(true, PublicScope, &target, &[]);
            assert_eq!(stamp.tier, with_setting, "{label}: on, public scope");
            if stamp.tier == Public {
                assert_eq!(
                    stamp.institutions, None,
                    "{label}: a public entry names none"
                );
            }
        }
        // A public chat's institutions never reach a public entry.
        let stamp = history_stamp(
            true,
            PublicScope,
            &chat(ChatReach::Public, ids(&["ucsf"])),
            &[],
        );
        assert_eq!(stamp.institutions, None);
    }

    /// The union of the chat's institutions and every private row's, with unknown failing
    /// toward a model on this computer only.
    #[test]
    fn the_institution_union() {
        let private = |target: &HistoryTarget, rows: &[Option<Vec<String>>]| {
            let stamp = history_stamp(false, BiasScope::Public, target, rows);
            assert_eq!(stamp.tier, EntryTier::Private);
            stamp.institutions
        };
        assert_eq!(private(&HistoryTarget::Outside, &[]), None);
        assert_eq!(
            private(&HistoryTarget::NewChat(ProviderTier::Private), &[None]),
            None
        );
        assert_eq!(
            private(&chat(ChatReach::Private, ids(&["ucsf"])), &[]),
            ids(&["ucsf"])
        );
        assert_eq!(
            private(
                &chat(ChatReach::Private, ids(&["ucsf"])),
                &[ids(&["Stanford"]), None, ids(&["ucsf"])]
            ),
            ids(&["stanford", "ucsf"]),
            "normalized, deduplicated and sorted"
        );
        assert_eq!(
            private(&HistoryTarget::Outside, &[ids(&["stanford"])]),
            ids(&["stanford"]),
            "a private row's institutions reach an entry outside any chat"
        );
        for (label, target, rows) in [
            (
                "chat unreadable",
                chat(ChatReach::Private, Some(vec![])),
                vec![],
            ),
            ("unreachable", HistoryTarget::Unreachable, vec![]),
            ("row unreadable", HistoryTarget::Outside, vec![Some(vec![])]),
            (
                "an id that does not normalize",
                HistoryTarget::Outside,
                vec![ids(&[""])],
            ),
            (
                "more than eight",
                chat(
                    ChatReach::Private,
                    Some((0..5).map(|i| format!("a{i}")).collect()),
                ),
                vec![Some((0..4).map(|i| format!("b{i}")).collect())],
            ),
        ] {
            assert_eq!(private(&target, &rows), Some(vec![]), "{label}");
        }
    }

    fn entry(tier: EntryTier, institutions: Option<Vec<String>>) -> HistoryEntry {
        HistoryEntry {
            id: store::new_entry_id(),
            created_at: DateTime::from_timestamp(1_795_707_552, 0).unwrap(),
            text: "EPAS1".to_string(),
            tier,
            institutions,
            surface: Surface::Composer,
            duration_ms: 1_000,
            model: "whisper-small.en-q8_0".to_string(),
            private_terms: false,
            session_id: None,
            session_incarnation: None,
            truncated: None,
        }
    }

    fn inst(id: &str) -> InstitutionId {
        InstitutionId::new(id)
    }

    /// The P-16 truth table, under each mixing policy: a public caller reads public
    /// entries only while the tiers are enforced; a private caller reads a private entry
    /// when every institution it stores admits the caller's model; `null` admits every
    /// private model, `[]` only a local one, and `{stanford, ucsf}` only a local one.
    /// `open` lifts the institution check; the switch off lifts both.
    #[test]
    fn the_agent_read_truth_table() {
        let public = entry(EntryTier::Public, None);
        let null = entry(EntryTier::Private, None);
        let ucsf_row = entry(EntryTier::Private, ids(&["ucsf"]));
        let pair_row = entry(EntryTier::Private, ids(&["stanford", "ucsf"]));
        let unknown = entry(EntryTier::Private, Some(vec![]));
        let rows = [
            ("public", &public),
            ("null", &null),
            ("ucsf", &ucsf_row),
            ("stanford and ucsf", &pair_row),
            ("unknown", &unknown),
        ];
        let local = Some(ModelAffiliation::Local);
        let ucsf = Some(ModelAffiliation::institution(inst("ucsf")));
        let stanford = Some(ModelAffiliation::institution(inst("stanford")));
        let both = ModelAffiliation::institutions([inst("ucsf"), inst("stanford")]);
        let every = vec!["public", "null", "ucsf", "stanford and ucsf", "unknown"];
        for policy in [
            MixingPolicy::Standard,
            MixingPolicy::Strict,
            MixingPolicy::Open,
        ] {
            let _pin = pin_for_test(policy);
            let open = policy == MixingPolicy::Open;
            let unless_open = |names: &[&'static str]| {
                if open {
                    every.clone()
                } else {
                    names.to_vec()
                }
            };
            let callers = [
                (
                    "public, enforced",
                    CallCapability::for_test(ProviderTier::Public, true),
                    vec!["public"],
                ),
                (
                    "public, switch off",
                    CallCapability::for_test(ProviderTier::Public, false),
                    every.clone(),
                ),
                (
                    "local",
                    CallCapability::for_test_affiliated(ProviderTier::Private, true, local),
                    every.clone(),
                ),
                (
                    "ucsf",
                    CallCapability::for_test_affiliated(ProviderTier::Private, true, ucsf),
                    unless_open(&["public", "null", "ucsf"]),
                ),
                (
                    "stanford",
                    CallCapability::for_test_affiliated(ProviderTier::Private, true, stanford),
                    unless_open(&["public", "null"]),
                ),
                (
                    "stanford and ucsf",
                    CallCapability::for_test_affiliated(ProviderTier::Private, true, both),
                    unless_open(&["public", "null"]),
                ),
                (
                    "unstated",
                    CallCapability::for_test_affiliated(ProviderTier::Private, true, None),
                    unless_open(&["public", "null"]),
                ),
                (
                    "stanford, switch off",
                    CallCapability::for_test_affiliated(ProviderTier::Private, false, stanford),
                    every.clone(),
                ),
            ];
            for (label, cap, want) in callers {
                let got: Vec<&str> = rows
                    .iter()
                    .filter(|(_, e)| agent_reads_dictation(e, cap))
                    .map(|(name, _)| *name)
                    .collect();
                assert_eq!(got, want, "{policy:?}, {label}");
            }
        }
    }

    /// The predicate reads only the capability it is handed: its production text never
    /// re-reads the master switch or samples the provider slot (Gate D on a tool call's
    /// path). The needles are assembled so this test's own text is not a match.
    #[test]
    fn the_predicates_read_only_what_they_are_handed() {
        let source = include_str!("history.rs");
        let production = source
            .split_once(concat!("#[cfg(test)]\n", "mod tests {"))
            .map_or(source, |(before, _)| before);
        for needle in [
            concat!("privacy_tiers", "_enabled"),
            concat!("CallCapability::", "sample"),
            concat!("public_", "enforced("),
        ] {
            assert!(
                !production.contains(needle),
                "history.rs must not spell {needle}: a tool call's reach is its admitted sample"
            );
        }
    }

    #[test]
    fn the_folder_is_the_local_data_folders() {
        assert_eq!(
            history_dir(),
            Paths::local_data_dir().join("dictation").join("history")
        );
        assert_eq!(open_history().dir(), history_dir());
        assert!(Arc::ptr_eq(&open_history(), &open_history()), "one handle");
    }
}
