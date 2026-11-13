use biorouter::dictation::extractor::{candidates, observe, Observation};
use biorouter::security::dictionary_store::store::{
    DictionaryStore, EntrySource, EntryStatus, EntryTier, RowAffiliation,
};

#[test]
fn extraction_requires_the_setting_and_two_chats() {
    let dir = tempfile::tempdir().unwrap();
    let dictionary = DictionaryStore::open(dir.path().to_path_buf());
    let item = |session_id, text| Observation {
        session_id,
        text,
        eligible: true,
        admitted_stamp: dictionary.view().unwrap().content_stamp(),
        tier: EntryTier::Public,
        affiliation: None,
    };
    let off_origin_stamp = dictionary.view().unwrap().content_stamp();
    observe(&dictionary, item("first", "EPAS1")).unwrap();
    assert!(dictionary.view().unwrap().entries().is_empty());
    dictionary
        .write(|file| {
            file.settings.learn_from_chats = true;
            Ok(())
        })
        .unwrap();
    observe(
        &dictionary,
        Observation {
            session_id: "off-origin",
            text: "EPAS1",
            eligible: true,
            admitted_stamp: off_origin_stamp,
            tier: EntryTier::Public,
            affiliation: None,
        },
    )
    .unwrap();
    assert!(dictionary.view().unwrap().entries().is_empty());
    observe(&dictionary, item("first", "EPAS1")).unwrap();
    observe(&dictionary, item("first", "EPAS1")).unwrap();
    let view = dictionary.view().unwrap();
    assert_eq!(view.entries()[0].seen, 1);
    assert_eq!(view.entries()[0].source, EntrySource::Auto);
    assert_eq!(view.entries()[0].status, EntryStatus::Suggested);
    observe(&dictionary, item("second", "EPAS1")).unwrap();
    assert_eq!(dictionary.view().unwrap().entries()[0].seen, 2);
    observe(&dictionary, item("first", "EPAS1")).unwrap();
    assert_eq!(dictionary.view().unwrap().entries()[0].seen, 2);
}

/// An ineligible observation, and one stamped Private, write nothing (owner, 2026-11-13:
/// suggestions come only from chats on public models). The control, the same text
/// stamped Public, lands.
#[test]
fn disabled_context_and_private_stamp_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let dictionary = DictionaryStore::open(dir.path().to_path_buf());
    dictionary
        .write(|file| {
            file.settings.learn_from_chats = true;
            Ok(())
        })
        .unwrap();
    observe(
        &dictionary,
        Observation {
            session_id: "excluded",
            text: "UCSF",
            eligible: false,
            admitted_stamp: dictionary.view().unwrap().content_stamp(),
            tier: EntryTier::Public,
            affiliation: None,
        },
    )
    .unwrap();
    assert!(dictionary.view().unwrap().entries().is_empty());
    observe(
        &dictionary,
        Observation {
            session_id: "private",
            text: "UCSF",
            eligible: true,
            admitted_stamp: dictionary.view().unwrap().content_stamp(),
            tier: EntryTier::Private,
            affiliation: Some(RowAffiliation {
                institutions: vec!["ucsf".into()],
            }),
        },
    )
    .unwrap();
    assert!(
        dictionary.view().unwrap().entries().is_empty(),
        "a private chat suggested a word"
    );
    observe(
        &dictionary,
        Observation {
            session_id: "public",
            text: "UCSF",
            eligible: true,
            admitted_stamp: dictionary.view().unwrap().content_stamp(),
            tier: EntryTier::Public,
            affiliation: None,
        },
    )
    .unwrap();
    let view = dictionary.view().unwrap();
    assert_eq!(view.entries().len(), 1, "control: a public chat suggests it");
    assert_eq!(view.entries()[0].tier, EntryTier::Public);
}

#[test]
fn candidate_filter_skips_protected_spans_and_person_identifiers() {
    assert_eq!(
        candidates("UCSF EPAS1 HIF2A OMOP"),
        ["UCSF", "EPAS1", "HIF2A"]
    );
    assert!(candidates("`UCSF` https://example.org/EPAS1 /tmp/HIF2A").is_empty());
    assert!(candidates("/dictionary add UCSF").is_empty());
}
