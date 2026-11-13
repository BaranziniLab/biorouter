//! Local, best-effort proposals from a person's own chat text.

use crate::conversation::message::{Message, ProvenanceKind};
use crate::guardrails::{pii::PiiDetector, secret_output};
use crate::privacy::{ProviderTier, SessionClassification};
use crate::security::dictionary_store::store;
use regex::Regex;
use rmcp::model::Role;
use std::ops::Range;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    LazyLock, OnceLock,
};
use std::time::Duration;

static CANDIDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:[A-Za-z]{1,10}[0-9][A-Za-z0-9]{0,10}|[A-Z]{3,}|[a-z]+[A-Z][A-Za-z0-9]*|[A-Z][a-z]+[A-Z][A-Za-z0-9]*)\b",
    )
    .expect("candidate regex")
});
static PROTECTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)```.*?```|`[^`]*`|https?://\S+|www\.\S+|(?:[A-Za-z]:)?[/\\][^\s]+|\b\S+@\S+\.\S+\b",
    )
    .expect("protected regex")
});
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static ADMITTED_STAMP: AtomicU64 = AtomicU64::new(0);
static WATCHER: OnceLock<()> = OnceLock::new();
const MAX_IN_FLIGHT: usize = 2;
/// How often the one watcher reads the store's settings.
const WATCH_INTERVAL: Duration = Duration::from_millis(100);
/// The most of one message the extractor copies or reads, in bytes. A pasted log or file
/// can be megabytes long, and every span and hit in it is work on a worker thread, so the
/// text is cut here before it is copied off the turn and again before it is scanned. A
/// term after the cut is not proposed from that message.
pub const MAX_OBSERVED_BYTES: usize = 16 * 1024;

/// `text` itself when it fits in [`MAX_OBSERVED_BYTES`], and otherwise its start up to
/// the last whitespace within the cap, so the cut splits no character, no token and no
/// link, path or address. A text whose first [`MAX_OBSERVED_BYTES`] hold no whitespace at
/// all is not prose, and nothing of it is kept.
pub fn capped(text: &str) -> &str {
    if text.len() <= MAX_OBSERVED_BYTES {
        return text;
    }
    let window = (0..=MAX_OBSERVED_BYTES)
        .rev()
        .find(|&at| text.is_char_boundary(at))
        .and_then(|end| text.get(..end))
        .unwrap_or_default();
    window
        .rfind(char::is_whitespace)
        .and_then(|at| window.get(..at))
        .unwrap_or_default()
}

/// Whether `message` holds words the person wrote and can see (DD-57), the rule PR-23's
/// `dictionary__add_terms` applies to the person's latest message. Tool results are stored
/// with the user role, and so are the rows only the model reads (hook context, post-edit
/// diagnostics, loop warnings, nudges, compaction summaries), which can echo text from
/// files or the web. So a message counts only when it is a user message the person can
/// see, carries text alone, and has no provenance or the provenance of the person's own
/// words: never `AgentInjection`, `SpawnContext` or any other writer's.
pub fn person_wrote(message: &Message) -> bool {
    message.role == Role::User
        && message.is_user_visible()
        && message.has_only_text_content()
        && message
            .metadata
            .provenance
            .as_ref()
            .is_none_or(|stamp| stamp.kind == ProvenanceKind::UserDirect)
}

/// The last ON version observed by the one background settings watcher. Zero
/// means OFF, unreadable, or not yet observed. The reply path does no I/O.
///
/// The watcher is a plain thread rather than a task, so it does not belong to the
/// runtime that happened to ask first: a task would stop with that runtime (every
/// `#[tokio::test]` has its own) and leave the stamp frozen for the rest of the
/// process. A thread that cannot be started leaves the stamp at zero, so nothing is
/// learned.
pub fn admission_stamp() -> Option<u64> {
    WATCHER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("dictionary-extractor-watch".into())
            .spawn(|| {
                let store = crate::security::dictionary_store::open_store();
                loop {
                    let stamp = store
                        .view()
                        .ok()
                        .and_then(|view| {
                            view.settings()
                                .learn_from_chats
                                .then_some(view.content_stamp())
                        })
                        .unwrap_or(0);
                    ADMITTED_STAMP.store(stamp, Ordering::Release);
                    std::thread::sleep(WATCH_INTERVAL);
                }
            });
    });
    let stamp = ADMITTED_STAMP.load(Ordering::Acquire);
    (stamp != 0).then_some(stamp)
}

pub struct Permit;

impl Drop for Permit {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::Release);
    }
}

pub fn try_reserve() -> Option<Permit> {
    IN_FLIGHT
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < MAX_IN_FLIGHT).then_some(n + 1)
        })
        .ok()
        .map(|_| Permit)
}

/// How many observations are queued or running now. A permit is taken before the turn
/// goes on and given back when the worker is done, so a caller that waits for zero has
/// given every queued observation its chance to land. The tests wait on it.
pub fn observations_in_flight() -> usize {
    IN_FLIGHT.load(Ordering::Acquire)
}

/// Whether a turn in `session` may teach the dictionary (DD-57): only a person's own chat
/// (`SessionType::User`), never a SubAgent, Scheduled, Hidden or Terminal session, and
/// never a chat a Crew grant restricts. A Crew registry that cannot be read counts as a
/// grant, as dispatch counts it.
pub async fn chat_may_learn(session: &crate::session::Session) -> bool {
    if session.session_type != crate::session::SessionType::User {
        return false;
    }
    match crate::crew::manager() {
        Ok(crew) => !crew.is_scoped_session(&session.id).await,
        Err(_) => false,
    }
}

/// The tier an observation is stamped with (DD-30, DD-57): Private unless
/// `chat_reach`, over the chat's stored tier and the tier of the provider its turn
/// used, answers Public.
pub fn stamp_tier(stored: SessionClassification, turn_provider: ProviderTier) -> store::EntryTier {
    if crate::dictation::chat_reach(Some(stored), Some(turn_provider)).stamp_private() {
        store::EntryTier::Private
    } else {
        store::EntryTier::Public
    }
}

const STOP: &[&str] = &[
    "API", "ASAP", "CSV", "FYI", "HTTP", "HTTPS", "JSON", "OK", "PDF", "PM", "TODO", "URL", "USA",
];

/// The caller must derive these fields from the admitted, persisted person message.
/// `eligible` includes roster, setting, policy, surface, session kind and provenance checks.
pub struct Observation<'a> {
    pub session_id: &'a str,
    pub text: &'a str,
    pub eligible: bool,
    pub admitted_stamp: u64,
    pub tier: store::EntryTier,
    pub affiliation: Option<store::RowAffiliation>,
}

/// Every span a candidate may not touch, found once per text: code, links, paths,
/// addresses, and what the PII detector flags. Sorted by start and merged, so no two
/// overlap and their ends rise with their starts.
fn skipped_spans(text: &str) -> Vec<Range<usize>> {
    let mut spans: Vec<Range<usize>> = PROTECTED.find_iter(text).map(|span| span.range()).collect();
    // A backtick no later one closes opens code that runs to the end of the text: a code
    // block the cap cut through, or one never closed. Only the last backtick can be such
    // an opener, because the protected pattern covers every earlier one, in a closed pair
    // or inside a link or path, so it alone is checked.
    if let Some(opener) = text.rfind('`') {
        if !spans.iter().any(|span| span.contains(&opener)) {
            spans.push(opener..text.len());
        }
    }
    spans.extend(
        PiiDetector::new()
            .scan(text)
            .into_iter()
            .map(|span| span.start..span.end),
    );
    spans.sort_unstable_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

/// At most three term shaped tokens from the first [`MAX_OBSERVED_BYTES`] of `text`, none
/// touching a skipped span. The hits come in order and the spans are sorted and disjoint,
/// so one index moves forward through the spans as the hits do: the work is one pass over
/// each, not every span for every hit.
pub fn candidates(text: &str) -> Vec<String> {
    let text = capped(text);
    if text.trim_start().starts_with('/') {
        return Vec::new();
    }
    let skipped = skipped_spans(text);
    let mut next_span = 0usize;
    proposals(
        CANDIDATE
            .find_iter(text)
            .filter(move |hit| {
                // A span that ends at or before this hit ends before every later hit too.
                while skipped
                    .get(next_span)
                    .is_some_and(|span| span.end <= hit.start())
                {
                    next_span += 1;
                }
                // Kept when no span is left, or the next one starts at or after its end.
                skipped
                    .get(next_span)
                    .is_none_or(|span| span.start >= hit.end())
            })
            .map(|hit| hit.as_str()),
    )
}

/// The first three of `words` that are not stop words, digits alone, refused by the
/// validator or shaped like a secret, once each by folded spelling.
fn proposals<'a>(words: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut found = Vec::new();
    for word in words {
        if STOP.contains(&word) || word.len() > 64 || word.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if store::validate::term(word).is_err() || secret_output::redact_text(word).is_some() {
            continue;
        }
        if !found
            .iter()
            .any(|seen: &String| store::fold_key(seen) == store::fold_key(word))
        {
            found.push(word.to_owned());
        }
        if found.len() == 3 {
            break;
        }
    }
    found
}

/// Runs on a blocking worker. Caller must not await this on the reply path.
///
/// Nothing is learned from a chat whose tier is Private (owner, 2026-11-13): suggestions
/// come only from chats on public models, judged by the chat's own tier (its stored tier
/// and the tier of the model its turn used, which counts decision models, DR-33), because
/// a private chat can hold patient data that no detector is sure to catch. This is the one
/// place both the live hooks and **Find words in my recent chats** pass through.
pub fn observe(
    store: &store::DictionaryStore,
    item: Observation<'_>,
) -> Result<(), store::StoreError> {
    if !item.eligible || item.tier.is_private() {
        return Ok(());
    }
    // Pure and bounded, so a message with nothing to propose never reads the store.
    let candidates = candidates(item.text);
    if candidates.is_empty() {
        return Ok(());
    }
    let admitted = store.view()?;
    if !admitted.settings().learn_from_chats || admitted.content_stamp() != item.admitted_stamp {
        return Ok(());
    }
    let at = store::schema::now();
    let _ = store.try_write_within_at_stamp(
        Duration::from_millis(50),
        item.admitted_stamp,
        |file| {
            if !file.settings.learn_from_chats {
                return Ok(());
            }
            for written in candidates {
                file.observe_auto_suggestion(
                    &written,
                    item.tier,
                    item.session_id,
                    item.affiliation.clone(),
                    at,
                )?;
            }
            Ok(())
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_queue_is_bounded() {
        let first = try_reserve().unwrap();
        let second = try_reserve().unwrap();
        assert!(try_reserve().is_none());
        drop(first);
        assert!(try_reserve().is_some());
        drop(second);
    }

    #[test]
    fn candidate_shapes_stop_words_protection_and_cap() {
        assert_eq!(
            candidates("EPAS1 HIF2A UCSF scRNAseq fourth3"),
            vec!["EPAS1", "HIF2A", "UCSF"]
        );
        assert!(candidates("`EPAS1` https://site.test/UCSF /tmp/HIF2A").is_empty());
        assert!(candidates("API JSON TODO").is_empty());
        assert!(candidates("/dictionary add EPAS1").is_empty());
    }

    /// PR-24 (DD-57): a message is cut to [`MAX_OBSERVED_BYTES`] at its last whitespace,
    /// so neither a character nor a token is split, and a text of no whitespace keeps
    /// nothing.
    #[test]
    fn a_long_message_is_cut_at_whitespace_within_the_cap() {
        let short = "EPAS1 and HIF2A";
        assert!(
            std::ptr::eq(capped(short), short),
            "a short text is kept whole"
        );

        // Seven bytes a unit, so the cap falls inside the second three byte character of
        // unit 2340, and the last whitespace before it ends unit 2339.
        let text = "€€ ".repeat(4_000);
        let kept = capped(&text);
        assert_eq!(kept.len(), 2_339 * 7 + 6);
        assert!(kept.len() <= MAX_OBSERVED_BYTES);
        assert!(text.starts_with(kept));
        assert!(kept.ends_with("€€"));

        let token = format!("EPAS1 {}", "Q".repeat(MAX_OBSERVED_BYTES));
        assert_eq!(capped(&token), "EPAS1", "a token is never cut in two");
        assert_eq!(capped(&"Q".repeat(MAX_OBSERVED_BYTES + 1)), "");
    }

    /// PR-24 (DD-57): a large text whose every hit sits in a protected span is read only
    /// to the cap, in one pass over its spans. Checking every span for every hit over the
    /// whole text, as `candidates` once did, takes far longer than the bound here. A term
    /// before the cap is still found and one after it is not.
    #[test]
    fn a_large_text_full_of_protected_spans_is_read_quickly_up_to_the_cap() {
        let text = format!("EPAS1 {} PASTCAP24", "`HIF2A` ".repeat(64 * 1024));
        assert!(text.len() > 32 * MAX_OBSERVED_BYTES);
        let started = std::time::Instant::now();
        let found = candidates(&text);
        let took = started.elapsed();
        assert_eq!(found, vec!["EPAS1"]);
        assert!(
            took < Duration::from_secs(2),
            "{took:?} for a text the cap bounds"
        );
    }

    /// PR-24 (DD-57): the sorted, merged sweep skips exactly what checking every hit
    /// against every span, unsorted and unmerged, skips: spans that nest, overlap, touch,
    /// and arrive out of order, because the PII spans are found after the protected ones.
    #[test]
    fn the_sweep_skips_what_checking_every_span_skips() {
        fn every_span(text: &str) -> Vec<String> {
            let skipped: Vec<Range<usize>> = PROTECTED
                .find_iter(text)
                .map(|span| span.range())
                .chain(
                    PiiDetector::new()
                        .scan(text)
                        .into_iter()
                        .map(|span| span.start..span.end),
                )
                .collect();
            proposals(
                CANDIDATE
                    .find_iter(text)
                    .filter(|hit| {
                        !skipped
                            .iter()
                            .any(|span| span.start < hit.end() && hit.start() < span.end)
                    })
                    .map(|hit| hit.as_str()),
            )
        }
        for text in [
            "MRN QX48213 then `HIF2A` and EPAS1",
            "EPAS1 a.person@site.test/UCSF `PHD2` IL6",
            "call 415-555-0134 about EPAS1 at https://site.test/HIF2A then PHD2 and IL6",
            "`EPAS1``HIF2A` PHD2 /tmp/IL6`UCSF` OMOP",
            "SSN 123-45-6789 EPAS1 ```fn HashMap``` scRNAseq",
        ] {
            assert_eq!(candidates(text), every_span(text), "{text}");
        }
    }

    /// A backtick that nothing closes protects the rest of the text, so code in a block the
    /// cap cut through is never proposed. Before the opener the text is read as ever.
    #[test]
    fn an_unclosed_code_block_protects_the_rest_of_the_text() {
        assert_eq!(candidates("EPAS1 ```rust HashMap getValue"), vec!["EPAS1"]);
        assert_eq!(candidates("EPAS1 `HIF2A PHD2"), vec!["EPAS1"]);
        let cut = format!(
            "EPAS1 ```rust\n{}\n```",
            "let HashMap = getValue(); ".repeat(2 * 1024)
        );
        assert!(cut.len() > MAX_OBSERVED_BYTES);
        assert_eq!(candidates(&cut), vec!["EPAS1"]);
    }

    /// PR-24 (DD-57): a message with no candidate returns before the store is read, so the
    /// worker pays no parse for it. The control, a message with one, reads the store.
    #[test]
    fn a_message_with_no_candidate_never_reads_the_store() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("dictionary");
        store::DictionaryStore::open(dir.clone())
            .write(|file| {
                file.settings.learn_from_chats = true;
                Ok(())
            })
            .unwrap();
        let admitted = store::DictionaryStore::open(dir.clone())
            .view()
            .unwrap()
            .content_stamp();
        let observe_with = |text: &str| {
            let dictionary = store::DictionaryStore::open(dir.clone());
            observe(
                &dictionary,
                Observation {
                    session_id: "chat",
                    text,
                    eligible: true,
                    admitted_stamp: admitted,
                    tier: store::EntryTier::Public,
                    affiliation: None,
                },
            )
            .unwrap();
            dictionary.store_parses()
        };
        assert_eq!(observe_with("nothing term shaped here"), 0);
        assert!(
            observe_with("EPAS1") > 0,
            "control: a candidate reads the store"
        );
    }

    /// PR-24 (DD-57): only a user message the person can see, of text alone, with no
    /// provenance or the person's own, counts. A row only the model reads (hook context, a
    /// nudge, a summary) is stored with the user role and never counts, as PR-23's tool
    /// rule reads it.
    #[test]
    fn only_a_visible_message_the_person_wrote_counts() {
        use crate::conversation::message::MessageProvenance;

        let stamped = |kind| {
            Message::user()
                .with_text("EPAS1")
                .with_provenance(MessageProvenance {
                    kind,
                    from_session_id: None,
                    from_session_name: None,
                })
        };
        assert!(person_wrote(&Message::user().with_text("EPAS1")));
        assert!(person_wrote(&stamped(ProvenanceKind::UserDirect)));
        assert!(
            !person_wrote(&Message::user().with_text("EPAS1").agent_only()),
            "a row only the model reads counted"
        );
        assert!(!person_wrote(
            &stamped(ProvenanceKind::UserDirect).agent_only()
        ));
        assert!(!person_wrote(&stamped(ProvenanceKind::AgentInjection)));
        assert!(!person_wrote(&stamped(ProvenanceKind::SpawnContext)));
        assert!(!person_wrote(&Message::assistant().with_text("EPAS1")));
        assert!(!person_wrote(&Message::user().with_tool_response(
            "call",
            Ok(rmcp::model::CallToolResult::success(vec![]))
        )));
    }

    #[test]
    fn a_stamp_is_private_unless_both_the_chat_and_its_turn_are_public() {
        use store::EntryTier;
        assert_eq!(
            stamp_tier(SessionClassification::Public, ProviderTier::Public),
            EntryTier::Public
        );
        assert_eq!(
            stamp_tier(SessionClassification::Public, ProviderTier::Private),
            EntryTier::Private
        );
        assert_eq!(
            stamp_tier(SessionClassification::Private, ProviderTier::Public),
            EntryTier::Private
        );
    }

    /// DD-57: a token `PiiDetector` flags is never proposed. A medical record number's
    /// identifier has a term's shape, and the same token is a candidate without its
    /// label, so the detector is what skips it.
    #[test]
    fn a_token_the_pii_detector_flags_is_skipped() {
        let text = "MRN QX48213 and EPAS1";
        let flagged = PiiDetector::new().scan(text);
        assert!(
            flagged
                .iter()
                .any(|span| text.get(span.start..span.end) == Some("QX48213")),
            "premise: the detector flags the record number: {flagged:?}"
        );
        let found = candidates(text);
        assert!(!found.iter().any(|word| word == "QX48213"), "{found:?}");
        assert!(found.iter().any(|word| word == "EPAS1"), "{found:?}");
        assert!(
            candidates("QX48213 and EPAS1")
                .iter()
                .any(|word| word == "QX48213"),
            "control: without its label the identifier is a candidate"
        );
    }

    /// DD-57: only a person's own chat teaches the dictionary. The same chat stops while a
    /// Crew grant restricts it, and no other kind of session ever does.
    #[tokio::test]
    async fn only_a_persons_own_chat_outside_crew_may_learn() {
        use crate::session::SessionType;

        let temp = tempfile::TempDir::new().unwrap();
        let sessions = crate::session::SessionManager::new(temp.path().to_path_buf());
        let person = sessions
            .create_session(
                temp.path().to_path_buf(),
                "person".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(
            chat_may_learn(&person).await,
            "premise: a person's own chat may learn"
        );
        let crew = crate::crew::install_test_scope(&person.id, None).await;
        let scoped = chat_may_learn(&person).await;
        crate::crew::remove_test_scope(&crew, &person.id).await;
        assert!(!scoped, "a Crew scoped chat may learn");
        assert!(
            chat_may_learn(&person).await,
            "the same chat without its scope may learn again, so the scope stopped it"
        );
        for kind in [
            SessionType::SubAgent,
            SessionType::Scheduled,
            SessionType::Hidden,
            SessionType::Terminal,
        ] {
            let other = sessions
                .create_session(temp.path().to_path_buf(), format!("{kind} session"), kind)
                .await
                .unwrap();
            assert!(!chat_may_learn(&other).await, "a {kind} session may learn");
        }
    }

    /// The chat extractor's store work at the cap (5,000 active rows) with two writers,
    /// the most the worker queue runs at once (PR-24, DD-57: p99 under 50 ms). Each
    /// observation opens its own handle and observes a new term, as the worker does, at
    /// the version a long lived handle last read, as the settings watcher reads it. So an
    /// observation that loses the lock waits up to the 50 ms budget, and one that finds
    /// the store changed since its admission is dropped. A timing run, by hand:
    /// `cargo test -p biorouter --lib dictation::extractor::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing run, by hand"]
    fn measure_an_observation_at_the_cap_with_two_writers() {
        use std::time::Instant;

        const WRITERS: usize = 2;
        const MESSAGES: usize = 200;
        let root = tempfile::tempdir().expect("a folder");
        let dir = root.path().join("dictionary");
        let at = store::schema::now();
        store::DictionaryStore::open(dir.clone())
            .write(|file| {
                file.settings.learn_from_chats = true;
                for i in 0..store::ops::MAX_ACTIVE_PER_TIER {
                    let written = format!("BRL{i:04}X");
                    file.entries.push(store::Entry {
                        id: store::EntryId::random(),
                        rev: 1,
                        match_policy: store::MatchPolicy::default_for(None, &written),
                        written,
                        tier: store::EntryTier::Public,
                        status: store::EntryStatus::Active,
                        source: store::EntrySource::Import,
                        reviewed: true,
                        spoken_forms: Vec::new(),
                        misrecognitions: Vec::new(),
                        kind: None,
                        bias: store::BiasPolicy::Auto,
                        context_cues: Vec::new(),
                        note: None,
                        reason: None,
                        suggested_in: None,
                        seen: 1,
                        seen_in: Vec::new(),
                        created_at: at,
                        updated_at: at,
                        provenance: None,
                        affiliation: None,
                    });
                }
                Ok(())
            })
            .unwrap();
        let bytes = std::fs::metadata(store::DictionaryStore::open(dir.clone()).store_path())
            .unwrap()
            .len();

        let writers: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let watcher = store::DictionaryStore::open(dir.clone());
                    let session = format!("writer-{writer}");
                    let mut samples = Vec::with_capacity(MESSAGES);
                    let mut landed = 0usize;
                    for i in 0..MESSAGES {
                        let term = format!("PRW{writer}T{i:04}X");
                        let text = format!("We looked at {term} today.");
                        let admitted = watcher.view().unwrap().content_stamp();
                        let started = Instant::now();
                        let dictionary = store::DictionaryStore::open(dir.clone());
                        observe(
                            &dictionary,
                            Observation {
                                session_id: &session,
                                text: &text,
                                eligible: true,
                                admitted_stamp: admitted,
                                tier: store::EntryTier::Public,
                                affiliation: None,
                            },
                        )
                        .unwrap();
                        samples.push(started.elapsed());
                        if dictionary
                            .view()
                            .unwrap()
                            .entries()
                            .iter()
                            .any(|row| row.written == term)
                        {
                            landed += 1;
                        }
                    }
                    (samples, landed)
                })
            })
            .collect();
        let mut samples: Vec<Duration> = Vec::new();
        let mut landed = 0usize;
        for writer in writers {
            let (mine, written) = writer.join().unwrap();
            samples.extend(mine);
            landed += written;
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        let p99 = samples[samples.len() * 99 / 100];
        let max = *samples.last().unwrap();
        println!(
            "{} rows, {bytes} bytes, {WRITERS} writers, {} observations: p50 {p50:?}, p99 \
             {p99:?}, max {max:?}; {landed} landed, the rest dropped on contention or a \
             changed store",
            store::ops::MAX_ACTIVE_PER_TIER,
            samples.len()
        );
        assert!(
            p99 < Duration::from_millis(50),
            "p99 {p99:?} is over the 50 ms target"
        );
    }
}
