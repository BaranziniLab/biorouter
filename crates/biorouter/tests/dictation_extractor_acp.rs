//! PR-24 (DD-57): a process that serves ACP over stdio learns nothing from its chats,
//! because the Jupyter persona delivers other people's messages as user messages. A
//! binary of its own: `run` (biorouter-acp) marks the process with
//! `set_process_surface(Surface::Acp)` before it serves anything, and the mark cannot be
//! undone, so it would end every other extractor test in the same process. The shared
//! helpers are in `dictation_extractor_support`.

mod dictation_extractor_support;

use biorouter::dictation::activation::{process_surface, set_process_surface, Surface};
use biorouter::session::SessionType;
use dictation_extractor_support::{
    learn_from_chats, rows_named, say, settled, wait_for, CountingProvider, Fixture, SERIAL,
};

/// Before the mark this process learns, as the control. After it, neither a new chat nor
/// the chat that learned suggests anything, and each turn still reaches the model.
#[tokio::test(flavor = "multi_thread")]
async fn a_process_marked_as_acp_stdio_suggests_nothing() {
    let _serial = SERIAL.lock().await;
    let fx = Fixture::scratch();
    let provider = CountingProvider::public();
    let agent = fx.agent().await;
    learn_from_chats(true);

    let earlier = fx.chat(SessionType::User).await;
    agent
        .update_provider(provider.clone(), &earlier.id)
        .await
        .unwrap();
    assert_eq!(
        process_surface(),
        Surface::Unspecified,
        "premise: nothing marked this process yet"
    );
    say(&agent, &earlier, "ACPCTL24", &provider).await;
    wait_for("a term from before the mark to land", || {
        rows_named("ACPCTL24").len() == 1
    })
    .await;

    set_process_surface(Surface::Acp);
    assert_eq!(process_surface(), Surface::Acp);
    let later = fx.chat(SessionType::User).await;
    agent
        .update_provider(provider.clone(), &later.id)
        .await
        .unwrap();
    for (chat, term) in [(&later, "ACPNEW24"), (&earlier, "ACPOLD24")] {
        say(&agent, chat, term, &provider).await;
        settled().await;
        assert!(
            rows_named(term).is_empty(),
            "an ACP stdio process suggested {term}"
        );
    }
    learn_from_chats(false);
}
