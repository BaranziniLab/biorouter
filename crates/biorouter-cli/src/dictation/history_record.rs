//! The terminal's half of the dictation history (DD-96; `docs/dictation/surfaces.md`,
//! "Dictation history"): what `biorouter dictate` and the terminal front ends (the TUI, the
//! classic REPL, a shared chat and `session attach`) record once their text is out.
//!
//! One entry per finished dictation a person at this terminal started. [`record`] takes
//! the [`CaptureConsent`] that opened the microphone, which only a person at an
//! interactive terminal with `BIOROUTER_TERMINAL` unset can hold, so an agent's shell never
//! records. Nothing is recorded for an empty text, while **Dictation history** is off, or
//! under managed `history: false`; `dictate --no-history` and `biorouter transcribe` never
//! call it. A failed append drops the entry with one fixed log line, and the dictation
//! itself never fails because of the history.
//!
//! The tier and institutions come from the core's `history_stamp`, which this file and the
//! daemon's stream route are the only callers of: private unless the person turned on
//! public by default and nothing about the dictation says private. Where the text went is
//! read in this process ([`DictatedInto`]): a chat named by its id, with its stored tier,
//! the institutions behind it and its incarnation, or no chat, for text that was printed
//! or copied.
//!
//! The history never feeds recognition (P-18): nothing here reads it.

use std::collections::BTreeSet;
use std::sync::Arc;

use biorouter::dictation::history::{
    self, history_stamp, HistoryEntry, HistoryStore, HistoryTarget, Surface,
};
use biorouter::dictation::recognize::BiasScope;
use biorouter::dictation::{chat_reach, ChatReach, DictionaryRequest};
use biorouter::managed::{ManagedPolicy, Rule};
use biorouter::privacy::affiliation::{dictionary_row_institutions, InstitutionId, WriterModel};
use biorouter::privacy::{ModelAffiliation, ProviderTier, SessionClassification};
use biorouter::security::dictionary_store::{self, store};
use chrono::{DateTime, Utc};

use super::consent::CaptureConsent;

/// One finished dictation, as the terminal produced it.
pub struct Dictation {
    /// The corrected text, as it went out.
    pub text: String,
    pub surface: Surface,
    /// The recording's length.
    pub duration_ms: u64,
    /// The speech model's catalog id.
    pub model: String,
    /// The dictionary the recording used, if any: its scope says whether private terms
    /// were eligible, and its private rows add their institutions.
    pub dictionary: Option<Arc<DictionaryRequest>>,
}

/// Where a terminal dictation's text went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictatedInto {
    /// Printed or copied: no chat.
    NoChat,
    /// A chat, named by its id, with the affiliation its live agent's model states:
    /// `None` when there is no live agent or the model states none, which reads as unknown.
    Chat {
        id: String,
        model: Option<ModelAffiliation>,
    },
}

impl DictatedInto {
    /// `dictate`'s `--session`, read offline: the text is for that chat, and no live agent
    /// says which model it runs on.
    pub fn from_session(session: Option<String>) -> Self {
        match session.filter(|id| !id.trim().is_empty()) {
            Some(id) => Self::Chat { id, model: None },
            None => Self::NoChat,
        }
    }
}

/// The target a new entry is stamped for, and the chat it may link to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub target: HistoryTarget,
    /// Kept only with the chat's incarnation, since a link is honoured only while the chat
    /// holding the id is still the one it was (ids can be reissued).
    pub session_id: Option<String>,
    pub session_incarnation: Option<i64>,
}

impl Destination {
    /// No chat: Crew, the clipboard, another app, the app preview or the terminal.
    pub fn outside() -> Self {
        Self {
            target: HistoryTarget::Outside,
            session_id: None,
            session_incarnation: None,
        }
    }

    /// Read `into` now, from this process's session store.
    async fn read(into: DictatedInto) -> Self {
        let DictatedInto::Chat { id, model } = into else {
            return Self::outside();
        };
        let manager = biorouter::session::SessionManager::instance();
        let Ok(session) = manager.get_session(&id, false).await else {
            return Self {
                target: HistoryTarget::Unreachable,
                session_id: None,
                session_incarnation: None,
            };
        };
        let touched = manager.session_affiliations(&id).await;
        let incarnation = manager.session_incarnation(&id).await.ok().flatten();
        Self {
            target: chat_target(session.privacy_tier, model, touched),
            session_id: incarnation.map(|_| id),
            session_incarnation: incarnation,
        }
    }
}

/// A chat the terminal could read, as a stamp target.
///
/// Its reach is the stored tier, raised to private when its model states an affiliation,
/// which only a private model does (a public model has none): a chat on a private model
/// may still be stored Public before its first turn ratchets it. Its institutions, for a
/// private entry, are a private dictionary row's for the same chat (O-35, P-16): those
/// behind its model and those whose connectors it touched. A model that states no
/// affiliation reads as unknown, which fails toward a model on this computer only, since
/// the terminal cannot tell an unstated affiliation from an unread one. A chat stored and
/// bound public held nothing private, so it adds none.
pub fn chat_target(
    stored: SessionClassification,
    model: Option<ModelAffiliation>,
    touched: anyhow::Result<BTreeSet<InstitutionId>>,
) -> HistoryTarget {
    let live = model.map(|_| ProviderTier::Private);
    let reach = chat_reach(Some(stored), live);
    let institutions = match reach {
        ChatReach::Public => None,
        ChatReach::Private | ChatReach::Unknown => {
            let writer = model.map_or(WriterModel::Unknown, |affiliation| {
                WriterModel::Known(Some(affiliation))
            });
            dictionary_row_institutions(writer, touched).stored_ids()
        }
    };
    HistoryTarget::Chat {
        reach,
        institutions,
    }
}

/// What became of a dictation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// One entry was appended.
    Kept,
    /// **Dictation history** is off, its settings could not be read, or the managed policy
    /// forbids it.
    Off,
    /// Nothing to keep: no text.
    Empty,
    /// The store refused or the lock stayed busy; the entry was dropped.
    Dropped,
}

/// Record `dictation` in this account's history, once its text is out. Only a person at
/// this terminal holds `person`.
pub async fn record(person: &CaptureConsent, dictation: Dictation, into: DictatedInto) -> Recorded {
    let _person_started_it = person;
    let policy = ManagedPolicy::load().dictation().history.clone();
    let store = history::open_history();
    if !keeps(&store, &policy) {
        return Recorded::Off;
    }
    let destination = Destination::read(into).await;
    tokio::task::spawn_blocking(move || {
        record_in(
            &store,
            &policy,
            dictation,
            &destination,
            || dictionary_store::open_store().view().ok(),
            Utc::now(),
        )
    })
    .await
    .unwrap_or(Recorded::Dropped)
}

/// Whether a new dictation would be kept: the policy allows it and the person's switch is
/// on (an unreadable settings file reads as off).
fn keeps(store: &HistoryStore, policy: &Rule) -> bool {
    policy.is_allowed() && store.settings().keep_history
}

/// [`record`]'s work on a given store, policy, dictionary and clock, so a test needs no
/// terminal. `dictionary_view` is read only when the dictation used a private row.
pub fn record_in(
    store: &HistoryStore,
    policy: &Rule,
    dictation: Dictation,
    destination: &Destination,
    dictionary_view: impl FnOnce() -> Option<Arc<store::StoreView>>,
    at: DateTime<Utc>,
) -> Recorded {
    if dictation.text.trim().is_empty() {
        return Recorded::Empty;
    }
    let settings = store.settings();
    if !policy.is_allowed() || !settings.keep_history {
        return Recorded::Off;
    }
    let public_by_default = settings.public_by_default;
    let dictionary = dictation.dictionary.as_deref();
    let scope = dictionary.map_or(BiasScope::Public, DictionaryRequest::scope);
    let rows = private_rows(dictionary, dictionary_view);
    let stamp = history_stamp(public_by_default, scope, &destination.target, &rows);
    let entry = HistoryEntry {
        id: history::store::new_entry_id(),
        created_at: at,
        text: dictation.text,
        tier: stamp.tier,
        institutions: stamp.institutions,
        surface: dictation.surface,
        duration_ms: dictation.duration_ms,
        model: dictation.model,
        private_terms: scope == BiasScope::Private,
        session_id: destination.session_id.clone(),
        session_incarnation: destination.session_incarnation,
        truncated: None,
    };
    match store.append(entry) {
        Ok(()) => Recorded::Kept,
        Err(_) => {
            tracing::warn!("dictation history entry dropped");
            Recorded::Dropped
        }
    }
}

/// The stored institutions of every private row in the dictation's bias set, each in the
/// row's own meaning; unknown (`[]`) for a row the dictionary no longer holds or when the
/// dictionary cannot be read. Empty when no private row was eligible.
fn private_rows(
    dictionary: Option<&DictionaryRequest>,
    view: impl FnOnce() -> Option<Arc<store::StoreView>>,
) -> Vec<Option<Vec<String>>> {
    let ids: Vec<&str> = dictionary
        .map(|dictionary| {
            dictionary
                .eligible
                .entries()
                .iter()
                .filter(|entry| entry.tier.is_private())
                .map(|entry| entry.id.as_str())
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Vec::new();
    }
    let view = view();
    ids.into_iter()
        .map(|id| {
            view.as_deref()
                .zip(store::EntryId::parse(id))
                .and_then(|(view, id)| view.find(&id))
                .map_or_else(
                    || Some(Vec::new()),
                    |row| row.affiliation.as_ref().map(|a| a.institutions.clone()),
                )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::dictation::history::{EntryTier, SettingsChange, TimeWindow};
    use biorouter::dictation::{bias_set, BiasChannels};
    use std::borrow::Cow;
    use store::{DictionaryStore, NewTerm, Reach, RowAffiliation};

    fn fresh() -> (tempfile::TempDir, HistoryStore) {
        let root = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(root.path().join("history"));
        (root, store)
    }

    fn dictation(text: &str, surface: Surface) -> Dictation {
        Dictation {
            text: text.to_string(),
            surface,
            duration_ms: 4_200,
            model: "whisper-small.en-q8_0".to_string(),
            dictionary: None,
        }
    }

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_795_707_552, 0).unwrap()
    }

    fn entries(store: &HistoryStore) -> Vec<HistoryEntry> {
        store
            .search(None, TimeWindow::all(), None, 100, &|_| true)
            .unwrap()
            .entries
    }

    fn keep(store: &HistoryStore, d: Dictation, destination: &Destination) -> Recorded {
        record_in(store, &Rule::Allowed, d, destination, || None, at())
    }

    /// `dictate` records `terminal`, `dictate --copy` records `clipboard`, each once, with
    /// the model, the length, private by default and no chat.
    #[test]
    fn a_dictation_is_kept_once_with_its_surface() {
        let (_root, store) = fresh();
        for surface in [Surface::Terminal, Surface::Clipboard] {
            assert_eq!(
                keep(
                    &store,
                    dictation("EPAS1 levels rose", surface),
                    &Destination::outside()
                ),
                Recorded::Kept
            );
        }
        let kept = entries(&store);
        assert_eq!(kept.len(), 2);
        let surfaces: Vec<Surface> = kept.iter().map(|e| e.surface).collect();
        assert_eq!(
            surfaces,
            [Surface::Clipboard, Surface::Terminal],
            "newest first"
        );
        for entry in &kept {
            assert_eq!(entry.text, "EPAS1 levels rose");
            assert_eq!(entry.tier, EntryTier::Private, "private by default");
            assert_eq!(entry.institutions, None);
            assert_eq!(entry.model, "whisper-small.en-q8_0");
            assert_eq!(entry.duration_ms, 4_200);
            assert!(!entry.private_terms);
            assert_eq!(entry.session_id, None);
            assert_eq!(entry.created_at, at());
        }
    }

    /// Nothing is kept for an empty text, with the switch off, under managed
    /// `history: false`, or when the settings file cannot be read.
    #[test]
    fn nothing_is_kept_when_the_history_is_off() {
        let (_root, store) = fresh();
        assert_eq!(
            keep(
                &store,
                dictation("  \n", Surface::Terminal),
                &Destination::outside()
            ),
            Recorded::Empty
        );
        let forbidden = Rule::Forbidden(Cow::Borrowed("Your organization turned this off."));
        assert_eq!(
            record_in(
                &store,
                &forbidden,
                dictation("EPAS1", Surface::Terminal),
                &Destination::outside(),
                || None,
                at()
            ),
            Recorded::Off
        );
        store
            .set_settings(SettingsChange {
                keep_history: Some(false),
                public_by_default: None,
            })
            .unwrap();
        assert_eq!(
            keep(
                &store,
                dictation("EPAS1", Surface::Terminal),
                &Destination::outside()
            ),
            Recorded::Off
        );
        assert!(entries(&store).is_empty());

        let (_root, unreadable) = fresh();
        std::fs::create_dir_all(unreadable.dir()).unwrap();
        std::fs::write(unreadable.settings_path(), b"{\"format\":\"nope\"}").unwrap();
        assert_eq!(
            keep(
                &unreadable,
                dictation("EPAS1", Surface::Terminal),
                &Destination::outside()
            ),
            Recorded::Off
        );
        assert!(!unreadable.history_path().exists());
    }

    /// With **Save new dictations as public** on, a dictation printed or copied is public;
    /// one into a private chat, one whose chat could not be read, and one that used private
    /// terms stay private.
    #[test]
    fn public_by_default_follows_the_stamp() {
        let (_root, store) = fresh();
        store
            .set_settings(SettingsChange {
                keep_history: None,
                public_by_default: Some(true),
            })
            .unwrap();
        let private_chat = Destination {
            target: chat_target(SessionClassification::Private, None, Ok(BTreeSet::new())),
            session_id: Some("20261126_3".into()),
            session_incarnation: Some(7),
        };
        let unreachable = Destination {
            target: HistoryTarget::Unreachable,
            session_id: None,
            session_incarnation: None,
        };
        keep(
            &store,
            dictation("one", Surface::Terminal),
            &Destination::outside(),
        );
        keep(&store, dictation("two", Surface::Terminal), &private_chat);
        keep(&store, dictation("three", Surface::Terminal), &unreachable);
        let tiers: Vec<(String, EntryTier)> = entries(&store)
            .into_iter()
            .map(|e| (e.text, e.tier))
            .collect();
        assert_eq!(
            tiers,
            [
                ("three".to_string(), EntryTier::Private),
                ("two".to_string(), EntryTier::Private),
                ("one".to_string(), EntryTier::Public),
            ]
        );
        let linked = entries(&store)
            .into_iter()
            .find(|e| e.text == "two")
            .unwrap();
        assert_eq!(linked.session_id.as_deref(), Some("20261126_3"));
        assert_eq!(linked.session_incarnation, Some(7));
        assert_eq!(
            linked.institutions,
            Some(vec![]),
            "a private chat whose model is unknown is read by local models only"
        );
    }

    /// A chat's reach and institutions, as the terminal reads them.
    #[test]
    fn a_chat_target_fails_toward_private() {
        let ucsf = ModelAffiliation::institution(InstitutionId::new("ucsf"));
        let touched = || -> anyhow::Result<BTreeSet<InstitutionId>> {
            Ok([InstitutionId::new("stanford")].into_iter().collect())
        };
        assert_eq!(
            chat_target(SessionClassification::Public, None, touched()),
            HistoryTarget::Chat {
                reach: ChatReach::Public,
                institutions: None
            },
            "a public chat on a model that states nothing adds no institution"
        );
        assert_eq!(
            chat_target(SessionClassification::Public, Some(ucsf), touched()),
            HistoryTarget::Chat {
                reach: ChatReach::Private,
                institutions: Some(vec!["stanford".into(), "ucsf".into()])
            },
            "an affiliated model is private before the first turn ratchets the chat"
        );
        assert_eq!(
            chat_target(
                SessionClassification::Private,
                Some(ModelAffiliation::Local),
                Ok(BTreeSet::new())
            ),
            HistoryTarget::Chat {
                reach: ChatReach::Private,
                institutions: None
            },
            "a local model adds none"
        );
        assert_eq!(
            chat_target(SessionClassification::Private, None, touched()),
            HistoryTarget::Chat {
                reach: ChatReach::Private,
                institutions: Some(vec![])
            },
            "an unknown model fails toward local models only"
        );
        assert_eq!(
            chat_target(
                SessionClassification::Private,
                Some(ucsf),
                Err(anyhow::anyhow!("unreadable"))
            ),
            HistoryTarget::Chat {
                reach: ChatReach::Private,
                institutions: Some(vec![])
            },
            "an unreadable touched record fails toward local models only"
        );
    }

    #[test]
    fn dictate_names_its_chat_only_with_a_session() {
        assert_eq!(DictatedInto::from_session(None), DictatedInto::NoChat);
        assert_eq!(
            DictatedInto::from_session(Some("  ".into())),
            DictatedInto::NoChat
        );
        assert_eq!(
            DictatedInto::from_session(Some("20261126_3".into())),
            DictatedInto::Chat {
                id: "20261126_3".into(),
                model: None
            }
        );
    }

    /// A recording that included the person's private terms is private and marked so, and
    /// carries the institutions of every private row its bias set held; a row that cannot
    /// be read any more fails toward local models only.
    #[test]
    fn private_terms_keep_an_entry_private_with_their_institutions() {
        let root = tempfile::tempdir().unwrap();
        let dictionary = DictionaryStore::open(root.path().join("dictionary"));
        dictionary
            .write(|file| {
                let mut ucsf = NewTerm::person("EPAS1", store::EntryTier::Private);
                ucsf.affiliation = Some(RowAffiliation {
                    institutions: vec!["ucsf".into()],
                });
                file.add(ucsf, Reach::All, store::schema::now())?;
                file.add(
                    NewTerm::person("PHD2", store::EntryTier::Private),
                    Reach::All,
                    store::schema::now(),
                )?;
                file.add(
                    NewTerm::person("UCSF", store::EntryTier::Public),
                    Reach::All,
                    store::schema::now(),
                )
            })
            .unwrap();
        let view = dictionary.view().unwrap();
        let (target, standing) = crate::dictation::consent::included_private_terms(
            crate::dictation::consent::PrivateTermsAsk {
                interactive: true,
                agent_shell: false,
                caller_private: true,
            },
        );
        let eligible = bias_set(&view, &target, standing, true);
        let request = Arc::new(DictionaryRequest::new(
            eligible,
            "",
            BiasChannels::for_dictation(),
        ));
        assert_eq!(request.scope(), BiasScope::Private);

        let mut rows = private_rows(Some(&request), || Some(Arc::clone(&view)));
        rows.sort();
        assert_eq!(rows, [None, Some(vec!["ucsf".to_string()])]);
        assert_eq!(
            private_rows(Some(&request), || None),
            [Some(vec![]), Some(vec![])],
            "an unreadable dictionary is unknown"
        );
        assert!(private_rows(None, || panic!("no dictionary, no read")).is_empty());

        let (_root, history) = fresh();
        history
            .set_settings(SettingsChange {
                keep_history: None,
                public_by_default: Some(true),
            })
            .unwrap();
        let mut d = dictation("EPAS1 and PHD2", Surface::Terminal);
        d.dictionary = Some(request);
        assert_eq!(
            record_in(
                &history,
                &Rule::Allowed,
                d,
                &Destination::outside(),
                || Some(Arc::clone(&view)),
                at()
            ),
            Recorded::Kept
        );
        let entry = entries(&history).remove(0);
        assert_eq!(entry.tier, EntryTier::Private);
        assert!(entry.private_terms);
        assert_eq!(entry.institutions, Some(vec!["ucsf".to_string()]));
    }

    /// `history_stamp` has one caller in this crate, this file, and `biorouter transcribe`
    /// never reaches the recorder: it converts a file the person already holds.
    #[test]
    fn only_this_file_stamps_and_transcribe_never_records() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut todo = vec![src.clone()];
        let mut stampers = Vec::new();
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    if text.contains(concat!("history_", "stamp(")) {
                        stampers.push(
                            path.strip_prefix(&src)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
            }
        }
        assert_eq!(stampers, ["dictation/history_record.rs"]);
        let transcribe = std::fs::read_to_string(src.join("commands/transcribe.rs")).unwrap();
        for needle in [concat!("history_", "record"), concat!("open_", "history")] {
            assert!(
                !transcribe.contains(needle),
                "transcribe must not record a dictation ({needle})"
            );
        }
    }
}
