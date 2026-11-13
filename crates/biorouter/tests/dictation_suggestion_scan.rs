//! **Find words in my recent chats** (owner, 2026-11-13): the chat extractor run once over
//! the person's stored chats, through `suggestion_scan::scan`, with a scratch session store
//! and a scratch Dictionary. No provider is called and no owner data is read.
//!
//! A Crew scoped chat is the lib test `a_crew_scoped_chat_is_never_scanned` in
//! `dictation/suggestion_scan.rs`, because the scope comes from a crate private helper.

mod dictation_extractor_support;

use std::sync::atomic::{AtomicBool, Ordering};

use biorouter::conversation::message::{Message, MessageProvenance, ProvenanceKind};
use biorouter::dictation::suggestion_scan::{
    mark_not_learnable, scan, NotLearnableReason, ScanProgress, ScanRefusal, ScanState,
    ACP_SESSION_NAME,
};
use biorouter::privacy::SessionClassification;
use biorouter::security::dictionary_store::{open_store, store};
use biorouter::session::{Session, SessionType};
use dictation_extractor_support::{learn_from_chats, rows_named, Fixture, SERIAL};

/// A chat of `kind` named `name` holding the person's `typed` messages, each followed by an
/// assistant reply that names `OMOPBOT24`, which a scan must never read.
async fn stored_chat(fx: &Fixture, kind: SessionType, name: &str, typed: &[&str]) -> Session {
    let chat = fx
        .sessions
        .create_session(std::env::temp_dir(), name.to_string(), kind)
        .await
        .unwrap();
    for text in typed {
        fx.sessions
            .add_message(&chat.id, &Message::user().with_text(*text))
            .await
            .unwrap();
        fx.sessions
            .add_message(
                &chat.id,
                &Message::assistant().with_text("OMOPBOT24 is fine"),
            )
            .await
            .unwrap();
    }
    fx.sessions.get_session(&chat.id, false).await.unwrap()
}

async fn run(fx: &Fixture) -> Result<ScanProgress, ScanRefusal> {
    let stop = AtomicBool::new(false);
    scan(&fx.sessions, &stop, &mut |_| {}).await
}

fn reset_dictionary() {
    open_store()
        .write(|file| {
            *file = store::StoreFile::default();
            Ok(())
        })
        .unwrap();
}

/// A scan needs the person's switch on, and with it off writes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_scan_needs_the_persons_switch() {
    let _serial = SERIAL.lock().await;
    reset_dictionary();
    let fx = Fixture::scratch();
    stored_chat(&fx, SessionType::User, "one", &["SWITCH24"]).await;
    stored_chat(&fx, SessionType::User, "two", &["SWITCH24"]).await;
    learn_from_chats(false);
    assert_eq!(run(&fx).await, Err(ScanRefusal::Off));
    assert!(rows_named("SWITCH24").is_empty());
}

/// Only the person's own words in their own chats on public models are read: an
/// assistant's reply, an agent's injected message, a private chat, an app's chat, a chat an
/// ACP client opened (marked, or still under its first name), a subagent's, a scheduled
/// run's, a terminal's and a hidden chat give nothing. A word typed in one chat is not
/// written; one typed in two is listed for review, public, as the extractor would list it.
#[tokio::test(flavor = "multi_thread")]
async fn a_scan_suggests_only_the_persons_words_from_public_chats() {
    let _serial = SERIAL.lock().await;
    reset_dictionary();
    let fx = Fixture::scratch();
    learn_from_chats(true);

    let first = stored_chat(
        &fx,
        SessionType::User,
        "first",
        &["SCANTWO24 and SCANONE24"],
    )
    .await;
    let second = stored_chat(&fx, SessionType::User, "second", &["more SCANTWO24"]).await;
    for chat in [&first, &second] {
        let injected = Message::user()
            .with_text("SCANINJ24")
            .with_provenance(MessageProvenance {
                kind: ProvenanceKind::AgentInjection,
                from_session_id: None,
                from_session_name: None,
            });
        fx.sessions.add_message(&chat.id, &injected).await.unwrap();
        fx.sessions
            .add_message(
                &chat.id,
                &Message::user().with_text("SCANMDL24").agent_only(),
            )
            .await
            .unwrap();
    }

    for name in ["private one", "private two"] {
        let chat = stored_chat(&fx, SessionType::User, name, &["SCANPRV24"]).await;
        fx.sessions
            .update(&chat.id)
            .raise_privacy(SessionClassification::Private, "fixture")
            .apply()
            .await
            .unwrap();
    }
    // Public by its stored tier, which ratchets only while privacy tiers are on, yet bound
    // to a model the registry calls Private, or having touched an institution.
    for name in ["local model one", "local model two"] {
        let chat = stored_chat(&fx, SessionType::User, name, &["SCANLOC24"]).await;
        fx.sessions
            .update(&chat.id)
            .provider_name("ollama")
            .apply()
            .await
            .unwrap();
    }
    for name in ["touched one", "touched two"] {
        let chat = stored_chat(&fx, SessionType::User, name, &["SCANINS24"]).await;
        fx.sessions
            .record_required_session_affiliation(
                &chat.id,
                biorouter::privacy::affiliation::InstitutionId::new("ucsf"),
            )
            .await
            .unwrap();
    }
    for name in ["app:notes:client-1", "app:notes:client-2"] {
        stored_chat(&fx, SessionType::User, name, &["SCANAPP24"]).await;
    }
    for name in ["Jupyter chat", "Zed chat"] {
        let chat = stored_chat(&fx, SessionType::User, name, &["SCANACP24"]).await;
        mark_not_learnable(&fx.sessions, &chat.id, NotLearnableReason::Acp).await;
    }
    stored_chat(&fx, SessionType::User, ACP_SESSION_NAME, &["SCANOLD24"]).await;
    stored_chat(&fx, SessionType::User, "a renamed chat", &["SCANOLD24"]).await;
    for kind in [
        SessionType::SubAgent,
        SessionType::Scheduled,
        SessionType::Terminal,
        SessionType::Hidden,
    ] {
        for n in 0..2 {
            stored_chat(&fx, kind, &format!("{kind} {n}"), &["SCANKND24"]).await;
        }
    }

    let progress = run(&fx).await.unwrap();
    assert_eq!(progress.state, ScanState::Finished);
    // The two person chats, the renamed one, and the four whose stored tier is Public but
    // whose model or institutions keep them out, which are listed and then not read.
    assert_eq!(
        progress.chats_total, 7,
        "the person chats, the renamed one and the four held back: {progress:?}"
    );
    assert_eq!(progress.chats_read, 7);
    assert_eq!(progress.words_found, 1);

    let rows = rows_named("SCANTWO24");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.source, store::EntrySource::Auto);
    assert_eq!(row.status, store::EntryStatus::Suggested);
    assert_eq!(row.tier, store::EntryTier::Public);
    assert_eq!(row.seen, 2, "listed for review");
    assert_eq!(row.seen_in.len(), 2);
    for term in [
        "SCANONE24",
        "OMOPBOT24",
        "SCANINJ24",
        "SCANMDL24",
        "SCANPRV24",
        "SCANAPP24",
        "SCANACP24",
        "SCANKND24",
        "SCANLOC24",
        "SCANINS24",
    ] {
        assert!(rows_named(term).is_empty(), "the scan suggested {term}");
    }
    // The chat under the ACP client's first name was skipped, so the word was seen in one
    // chat only.
    assert!(rows_named("SCANOLD24").is_empty());
    learn_from_chats(false);
}

/// A second scan of the same chats counts nothing twice, and a word the person dismissed
/// is not suggested again.
#[tokio::test(flavor = "multi_thread")]
async fn a_scan_counts_each_chat_once_and_never_brings_back_a_dismissed_word() {
    let _serial = SERIAL.lock().await;
    reset_dictionary();
    let fx = Fixture::scratch();
    learn_from_chats(true);
    for name in ["one", "two"] {
        stored_chat(&fx, SessionType::User, name, &["AGAIN24", "DROP24"]).await;
    }
    assert_eq!(run(&fx).await.unwrap().words_found, 2);
    let again = run(&fx).await.unwrap();
    assert_eq!(again.words_found, 0, "nothing new: {again:?}");
    assert_eq!(rows_named("AGAIN24")[0].seen_in.len(), 2);

    let drop = rows_named("DROP24")[0].id.clone();
    open_store()
        .write(|file| {
            file.dismiss_suggestions(&[drop], store::schema::now());
            Ok(())
        })
        .unwrap();
    assert_eq!(run(&fx).await.unwrap().words_found, 0);
    assert!(
        rows_named("DROP24").is_empty(),
        "a dismissed word came back"
    );
    let learning = open_store().view().unwrap().file().chat_learning.clone();
    assert_eq!((learning.kept, learning.dismissed), (0, 1));
    learn_from_chats(false);
}

/// Stopping keeps what the chats read so far hold, and reads no more of them.
#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_scan_keeps_what_it_found_so_far() {
    let _serial = SERIAL.lock().await;
    reset_dictionary();
    let fx = Fixture::scratch();
    learn_from_chats(true);
    for n in 0..4 {
        stored_chat(&fx, SessionType::User, &format!("chat {n}"), &["STOPWD24"]).await;
    }
    let stop = AtomicBool::new(false);
    let mut seen: Vec<ScanProgress> = Vec::new();
    let progress = scan(&fx.sessions, &stop, &mut |progress| {
        seen.push(progress.clone());
        if progress.chats_read == 2 {
            stop.store(true, Ordering::Release);
        }
    })
    .await
    .unwrap();
    assert_eq!(progress.state, ScanState::Stopped);
    assert_eq!((progress.chats_read, progress.chats_total), (2, 4));
    assert_eq!(progress.words_found, 1);
    assert_eq!(rows_named("STOPWD24")[0].seen_in.len(), 2);
    assert!(
        seen.iter()
            .any(|p| p.state == ScanState::Running && p.words_found == 1),
        "the running count showed the word before the scan ended: {seen:?}"
    );
    learn_from_chats(false);
}

/// When the person has dismissed most suggestions, a scan asks for more chats too.
#[tokio::test(flavor = "multi_thread")]
async fn a_scan_follows_the_persons_back_off() {
    let _serial = SERIAL.lock().await;
    reset_dictionary();
    let fx = Fixture::scratch();
    open_store()
        .write(|file| {
            for _ in 0..20 {
                file.chat_learning.record(false);
            }
            Ok(())
        })
        .unwrap();
    learn_from_chats(true);
    assert!(
        open_store()
            .view()
            .unwrap()
            .file()
            .chat_learning
            .required_chats()
            > 2,
        "premise: most were dismissed"
    );
    for name in ["one", "two"] {
        stored_chat(&fx, SessionType::User, name, &["BACKOFF24"]).await;
    }
    assert_eq!(run(&fx).await.unwrap().words_found, 0);
    assert_eq!(rows_named("BACKOFF24")[0].seen, 1, "held back at two chats");
    for name in ["three", "four", "five"] {
        stored_chat(&fx, SessionType::User, name, &["BACKOFF24"]).await;
    }
    assert_eq!(run(&fx).await.unwrap().words_found, 1);
    assert_eq!(rows_named("BACKOFF24")[0].seen, 2);
    learn_from_chats(false);
}
