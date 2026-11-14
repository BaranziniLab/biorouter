//! PR-24 (DD-57): the chat extractor through `Agent::reply`, the entry every desktop and
//! CLI consumer awaits, with a synthetic provider and a scratch store. No real provider
//! and no owner data. The shared helpers, and the reason for their waits, are in
//! `dictation_extractor_support`.
//!
//! Two exclusions are tested elsewhere. ACP stdio is `dictation_extractor_acp.rs`, a
//! binary of its own, because marking a process as ACP cannot be undone. A Crew scoped
//! chat is the lib test `a_crew_scoped_chat_queues_no_dictionary_suggestion` in
//! `agents/agent.rs`: such a chat's turn cannot reach a model without a live workspace
//! broker, and the scope it needs comes from a crate private test helper.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod dictation_extractor_support;

use biorouter::conversation::message::{Message, MessageProvenance, ProvenanceKind};
use biorouter::dictation::extractor::{admission_stamp, observations_in_flight};
use biorouter::privacy::affiliation::InstitutionId;
use biorouter::security::dictionary_store::store::{EntrySource, EntryStatus, EntryTier};
use biorouter::session::SessionType;
use dictation_extractor_support::{
    learn_from_chats, ready, reply, rows_named, say, settled, wait_for, CountingProvider, Fixture,
    SERIAL,
};

/// AgentInjection and SpawnContext provenance, a message only the model reads, SubAgent,
/// Scheduled, Terminal and Hidden sessions, an app's agent and an ACP `--ws` agent produce
/// no suggestion, while the same agent in a person's own chat, under the same waits, does.
/// Every turn reaches the model, so each absence is the extractor's decision.
#[tokio::test(flavor = "multi_thread")]
async fn injected_messages_and_other_kinds_of_session_suggest_nothing() {
    let _serial = SERIAL.lock().await;
    let fx = Fixture::scratch();
    let provider = CountingProvider::public();
    let agent = fx.agent().await;
    learn_from_chats(true);

    // Session kinds: the worker reads the session after the observation was queued.
    for (kind, term) in [
        (SessionType::SubAgent, "XSUBA24"),
        (SessionType::Scheduled, "XSCHD24"),
        (SessionType::Terminal, "XTERM24"),
        (SessionType::Hidden, "XHIDN24"),
    ] {
        let chat = fx.chat(kind).await;
        agent
            .update_provider(provider.clone(), &chat.id)
            .await
            .unwrap();
        say(&agent, &chat, term, &provider).await;
        settled().await;
        assert!(
            rows_named(term).is_empty(),
            "a {kind} session suggested {term}"
        );
    }

    // Provenance: the reply path copies only words the person wrote.
    let person = fx.chat(SessionType::User).await;
    agent
        .update_provider(provider.clone(), &person.id)
        .await
        .unwrap();
    for (kind, term) in [
        (ProvenanceKind::AgentInjection, "XINJT24"),
        (ProvenanceKind::SpawnContext, "XSPWN24"),
    ] {
        ready().await;
        let message = Message::user()
            .with_text(term)
            .with_provenance(MessageProvenance {
                kind,
                from_session_id: None,
                from_session_name: None,
            });
        assert!(
            reply(&agent, &person, message, &provider).await > 0,
            "the {kind:?} turn never reached the model"
        );
        settled().await;
        assert!(
            rows_named(term).is_empty(),
            "a message stamped {kind:?} suggested {term}"
        );
    }

    // Visibility: a row only the model reads (hook context, a nudge, a summary) is stored
    // with the user role and no provenance, and still is not the person's words.
    ready().await;
    let model_only = Message::user().with_text("XMODL24").agent_only();
    assert!(
        reply(&agent, &person, model_only, &provider).await > 0,
        "the model only turn never reached the model"
    );
    settled().await;
    assert!(
        rows_named("XMODL24").is_empty(),
        "a row only the model reads suggested XMODL24"
    );

    // Agents with dictionary learning off. `configure_agent` (routes/apps.rs) turns it off
    // for an app's main and worker agents, and `deny_stdio_mcp` (biorouter-acp) for an ACP
    // `--ws` agent; their own tests pin that they do.
    for (surface, term) in [("an app", "XAPPS24"), ("an ACP --ws", "XACPW24")] {
        let other = fx.agent().await;
        other.set_dictionary_learning(false);
        let chat = fx.chat(SessionType::User).await;
        other
            .update_provider(provider.clone(), &chat.id)
            .await
            .unwrap();
        say(&other, &chat, term, &provider).await;
        settled().await;
        assert!(
            rows_named(term).is_empty(),
            "{surface} agent suggested {term}"
        );
    }

    // The control: a person's own chat, the same agent, the same waits.
    say(&agent, &person, "XPERS24", &provider).await;
    wait_for("the person's own term to land", || {
        rows_named("XPERS24").len() == 1
    })
    .await;
    let row = rows_named("XPERS24").remove(0);
    assert_eq!(row.source, EntrySource::Auto);
    assert_eq!(row.status, EntryStatus::Suggested);
    assert_eq!(row.tier, EntryTier::Public);
    assert_eq!(row.seen, 1);
    assert_eq!(row.suggested_in.as_deref(), Some(person.id.as_str()));
    learn_from_chats(false);
}

/// A chat whose tier is Private suggests nothing (owner, 2026-11-13): a model on this
/// computer in a chat that touched no institution, and the same model in a chat whose
/// record holds `ucsf`. The control, the same agent in a chat on a public model, does.
/// Every turn reaches the model, so each absence is the extractor's decision.
#[tokio::test(flavor = "multi_thread")]
async fn a_private_chat_suggests_nothing() {
    let _serial = SERIAL.lock().await;
    let fx = Fixture::scratch();
    let local = CountingProvider::local();
    let public = CountingProvider::public();
    let agent = fx.agent().await;
    learn_from_chats(true);

    let untouched = fx.chat(SessionType::User).await;
    let touched = fx.chat(SessionType::User).await;
    fx.sessions
        .record_required_session_affiliation(&touched.id, InstitutionId::new("ucsf"))
        .await
        .unwrap();
    for (chat, term) in [(&untouched, "PRVNUL24"), (&touched, "PRVUCF24")] {
        agent
            .update_provider(local.clone(), &chat.id)
            .await
            .unwrap();
        say(&agent, chat, term, &local).await;
        settled().await;
        assert!(
            rows_named(term).is_empty(),
            "a private chat suggested {term}"
        );
    }

    let open = fx.chat(SessionType::User).await;
    agent
        .update_provider(public.clone(), &open.id)
        .await
        .unwrap();
    say(&agent, &open, "PUBCTL24", &public).await;
    wait_for("the public chat's term to land", || {
        rows_named("PUBCTL24").len() == 1
    })
    .await;
    assert_eq!(rows_named("PUBCTL24")[0].tier, EntryTier::Public);
    learn_from_chats(false);
}

/// The person's switch alone admits an observation: a chat whose agent does not have the
/// `dictionary` capability on still suggests the person's words (owner, 2026-11-13), and
/// with the switch off the same chat suggests nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_persons_switch_needs_no_dictionary_capability_in_the_chat() {
    let _serial = SERIAL.lock().await;
    let fx = Fixture::scratch();
    let provider = CountingProvider::public();
    let agent = fx.agent_without_the_capability();
    let chat = fx.chat(SessionType::User).await;
    agent
        .update_provider(provider.clone(), &chat.id)
        .await
        .unwrap();

    learn_from_chats(false);
    wait_for("the settings watcher to read the switch off", || {
        admission_stamp().is_none() && observations_in_flight() == 0
    })
    .await;
    assert!(
        reply(
            &agent,
            &chat,
            Message::user().with_text("NOCAPOFF24"),
            &provider
        )
        .await
            > 0,
        "the turn never reached the model"
    );
    settled().await;
    assert!(rows_named("NOCAPOFF24").is_empty(), "the switch was off");

    learn_from_chats(true);
    say(&agent, &chat, "NOCAPON24", &provider).await;
    wait_for("the term to land without the capability", || {
        rows_named("NOCAPON24").len() == 1
    })
    .await;
    learn_from_chats(false);
}

/// A token `PiiDetector` flags is skipped while the rest of the message is still read,
/// and `seen` counts a chat once however often the person writes the term there.
#[tokio::test(flavor = "multi_thread")]
async fn a_flagged_token_is_skipped_and_a_term_counts_each_chat_once() {
    let _serial = SERIAL.lock().await;
    let fx = Fixture::scratch();
    let provider = CountingProvider::public();
    let agent = fx.agent().await;
    learn_from_chats(true);

    let first = fx.chat(SessionType::User).await;
    let second = fx.chat(SessionType::User).await;
    for chat in [&first, &second] {
        agent
            .update_provider(provider.clone(), &chat.id)
            .await
            .unwrap();
    }

    say(&agent, &first, "MRN QX48213 and PIIOK24", &provider).await;
    wait_for("the unflagged term to land", || {
        rows_named("PIIOK24").len() == 1
    })
    .await;
    settled().await;
    assert!(
        rows_named("QX48213").is_empty(),
        "the record number the PII detector flags was suggested"
    );

    for (chat, seen) in [
        (&first, 1),
        (&first, 1),
        (&second, 2),
        (&first, 2),
        (&second, 2),
    ] {
        say(&agent, chat, "SEENTWO24", &provider).await;
        wait_for("the sighting to be read", || {
            rows_named("SEENTWO24")
                .first()
                .is_some_and(|row| row.seen >= seen)
        })
        .await;
        settled().await;
        let rows = rows_named("SEENTWO24");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seen, seen, "after a sighting in {}", chat.id);
    }
    learn_from_chats(false);
}
