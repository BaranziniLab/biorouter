//! Concurrency stress for the conversation-writeback freshness discipline.
//!
//! `conversation_writeback_freshness.rs` proves each caller does the right
//! thing, deterministically, one race at a time. This file does the opposite:
//! it puts the STORE under real, unsynchronized load — many tasks appending to
//! one session on a multi-threaded runtime while other tasks snapshot,
//! "summarize" and write the whole history back — and asserts the invariants
//! that a DELETE+re-INSERT rewrite can break:
//!
//! 1. **No append is ever destroyed.** The whole point of the fix.
//! 2. **No message is ever duplicated.** The rewrite path has no
//!    duplicate-uid recovery of its own, so a collision on
//!    `UNIQUE(session_id, msg_uid)` would surface as an error, not a re-mint.
//! 3. **`msg_uid` is stable.** An id, once accepted, keeps naming the same
//!    message forever — across any number of rewrites (BR-45).
//! 4. **No rewrite fails.** A `database is locked` here means the rewrite
//!    transaction read before it wrote (see the write-first `UPDATE sessions`
//!    in `replace_conversation_inner`); under WAL that returns
//!    `SQLITE_BUSY_SNAPSHOT` immediately, bypassing the busy timeout.
//! 5. **The FTS recall mirror does not drift.** A surviving message stays
//!    findable; a message a compaction removed stops being findable.
//! 6. **BR-7 blob payloads survive.** A recovered tail row is re-inserted with
//!    its stub verbatim, so its payload must neither be swept nor duplicated.
//! 7. **Reads are never torn.** A concurrent reader never observes a partial
//!    rewrite.
//! 8. **A basis never outlives the history it describes.** A session id freed
//!    by a wipe is handed straight back out, so "same id, same rowids" is not
//!    proof of the same conversation.
//!
//! Tests 1-6 drive the store directly, so the only thing under test is the
//! write path. Tests 7 and 9 close the loop through the REAL reply loop with a
//! mock provider: a live turn auto-compacts, or recovers from two overflows,
//! while other writers append. No network in either case.
//!
//! The control (`the_unguarded_rewrite_still_destroys_concurrent_appends`) is
//! what makes the rest trustworthy — it asserts the workload is genuinely
//! lossy against an unguarded write-back, so a green run here means the guard
//! worked rather than that the race never happened.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent, ToolResponse};
use biorouter::conversation::Conversation;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::{ReplaceOutcome, SessionType, DB_NAME, SESSIONS_FOLDER};
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolResult, Content, Tool};
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn user(id: &str, text: &str) -> Message {
    Message::user().with_text(text).with_id(id)
}

fn assistant(id: &str, text: &str) -> Message {
    Message::assistant().with_text(text).with_id(id)
}

/// A tool response comfortably over the 64 KiB BR-7 externalization threshold,
/// so storing it mints a `message_blobs` row and leaves a stub in `messages`.
fn blob_message(id: &str, marker: &str) -> Message {
    let text: String = (0..3_000)
        .map(|i| format!("{marker} row {i} of a very large tool result\n"))
        .collect();
    Message::assistant()
        .with_content(MessageContent::ToolResponse(ToolResponse {
            id: format!("call-{id}"),
            tool_result: Ok(CallToolResult {
                content: vec![Content::text(text)],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
            metadata: None,
        }))
        .with_id(id)
}

/// The full text of a message, tool responses included, as one string.
fn text_of(message: &Message) -> String {
    message
        .content
        .iter()
        .map(|item| match item {
            MessageContent::Text(t) => t.text.clone(),
            MessageContent::ToolResponse(r) => r
                .tool_result
                .as_ref()
                .map(|res| {
                    res.content
                        .iter()
                        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                        .collect::<String>()
                })
                .unwrap_or_default(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("")
}

async fn seeded_session(sm: &SessionManager, name: &str) -> String {
    let session = sm
        .create_session(PathBuf::from("/tmp/stress"), name.into(), SessionType::User)
        .await
        .unwrap();
    sm.add_message(&session.id, &user("seed-0", "seed message zero"))
        .await
        .unwrap();
    session.id
}

async fn stored(sm: &SessionManager, id: &str) -> Conversation {
    sm.get_session(id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
}

fn ids_of(conversation: &Conversation) -> Vec<String> {
    conversation
        .messages()
        .iter()
        .filter_map(|m| m.id.clone())
        .collect()
}

/// Assert no `msg_uid` appears twice in one stored conversation.
fn assert_no_duplicates(conversation: &Conversation, context: &str) {
    let ids = ids_of(conversation);
    let mut seen = HashSet::new();
    let dups: Vec<&String> = ids.iter().filter(|id| !seen.insert(*id)).collect();
    assert!(
        dups.is_empty(),
        "{context}: duplicate msg_uid stored: {dups:?}"
    );
    assert_eq!(
        ids.len(),
        conversation.len(),
        "{context}: a stored message lost its msg_uid"
    );
}

/// Open the store's SQLite file directly, read-only, for the invariants the
/// public API cannot express (FTS mirror rows, blob rows).
async fn probe(dir: &TempDir) -> sqlx::SqlitePool {
    let path = dir.path().join(SESSIONS_FOLDER).join(DB_NAME);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=ro", path.display()))
        .await
        .unwrap()
}

/// A whole-history rewrite holds SQLite's single write lock for the length of
/// an O(history) transaction — measured at ~310 ms for 300 messages in a debug
/// build, and identical with and without the freshness guard (312.5 ms vs
/// 312.6 ms p50 over 30 rewrites of the same history). A rewriter looping with
/// a 2 ms gap therefore re-takes the lock faster than a starved appender's 5 s
/// `busy_timeout` can expire, and roughly 1 append in 300 fails with
/// `SQLITE_BUSY`. That is pre-existing (it reproduces on the unguarded
/// `replace_conversation` path too) and it is NOT data loss: `add_message`
/// returns `Err`, so the caller is told the write failed.
///
/// The invariant these tests defend is the one BR-71 needs — every append the
/// store ACKNOWLEDGED is still there — so a busy append is tolerated and
/// counted rather than panicked on. Anything else is a hard failure.
fn is_busy(error: &str) -> bool {
    error.contains("database is locked")
}

/// One rewrite cycle in the shape every real caller uses: snapshot with its
/// revision, take time to "summarize", write back under the guard.
struct RewriteReport {
    outcomes: Vec<ReplaceOutcome>,
    errors: Vec<String>,
    /// Uids this rewriter deliberately dropped (compaction summarizing them
    /// away). The ONLY legitimate reason a stored message disappears.
    dropped: HashSet<String>,
}

// ── 1. many appenders, one rewriter: nothing may be lost ─────────────────────

/// The defect, at full pressure: 6 tasks appending 50 messages each while a
/// summarizer repeatedly snapshots the history, takes time, and writes the
/// whole thing back.
///
/// The rewriter is NON-destructive (it keeps everything it saw and adds a
/// summary), so the invariant is absolute: every one of the 300 appends must be
/// on disk at the end. Anything missing was destroyed by a stale write-back.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn three_hundred_concurrent_appends_survive_sixty_rewrites() {
    const APPENDERS: usize = 6;
    const PER_APPENDER: usize = 50;
    const REWRITES: usize = 60;

    let temp = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let id = seeded_session(&sm, "stress-1").await;

    let torn = Arc::new(Mutex::new(Vec::<String>::new()));
    let reads = Arc::new(AtomicUsize::new(0));
    let busy = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut appenders = Vec::new();
    for a in 0..APPENDERS {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        let busy = Arc::clone(&busy);
        appenders.push(tokio::spawn(async move {
            let mut written = Vec::new();
            for i in 0..PER_APPENDER {
                let uid = format!("append-{a}-{i}");
                let text = format!("appended message {a}/{i}");
                match sm.add_message(&id, &user(&uid, &text)).await {
                    // Acknowledged: the caller was told this landed, so it must
                    // still be there at the end. This is the whole invariant.
                    Ok(effective) => written.push((uid, effective, text)),
                    Err(e) if is_busy(&e.to_string()) => busy.lock().unwrap().push(uid),
                    Err(e) => panic!("append {uid} failed for a non-lock reason: {e}"),
                }
                tokio::task::yield_now().await;
            }
            written
        }));
    }

    let rewriter = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        tokio::spawn(async move {
            let mut report = RewriteReport {
                outcomes: Vec::new(),
                errors: Vec::new(),
                dropped: HashSet::new(),
            };
            for i in 0..REWRITES {
                let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
                let known = session.conversation.unwrap();
                // Stand-in for the summarization round-trip.
                tokio::time::sleep(Duration::from_millis(2)).await;
                let mut msgs = known.messages().clone();
                msgs.push(assistant(&format!("summary-{i}"), &format!("summary {i}")));
                match sm
                    .replace_conversation_preserving_tail(
                        &id,
                        &Conversation::new_unvalidated(msgs),
                        basis,
                        &known,
                    )
                    .await
                {
                    Ok((outcome, _)) => report.outcomes.push(outcome),
                    Err(e) => report.errors.push(e.to_string()),
                }
            }
            report
        })
    };

    // A concurrent reader, to catch a rewrite that is ever observable half-done.
    let reader = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        let torn = Arc::clone(&torn);
        let reads = Arc::clone(&reads);
        tokio::spawn(async move {
            for _ in 0..400 {
                let conversation = stored(&sm, &id).await;
                reads.fetch_add(1, Ordering::Relaxed);
                if conversation.is_empty() {
                    torn.lock()
                        .unwrap()
                        .push("observed an empty history".into());
                }
                let ids = ids_of(&conversation);
                let mut seen = HashSet::new();
                if let Some(dup) = ids.iter().find(|id| !seen.insert(*id)) {
                    torn.lock()
                        .unwrap()
                        .push(format!("observed a duplicate uid mid-flight: {dup}"));
                }
                tokio::task::yield_now().await;
            }
        })
    };

    let mut appended: Vec<(String, String, String)> = Vec::new();
    for handle in appenders {
        appended.extend(handle.await.unwrap());
    }
    let report = rewriter.await.unwrap();
    reader.await.unwrap();

    // 4. The write-first lock ordering held.
    assert!(
        report.errors.is_empty(),
        "no rewrite may fail (a `database is locked` here means the rewrite \
         transaction read before it wrote): {:?}",
        report.errors
    );
    assert_eq!(report.outcomes.len(), REWRITES);
    // Appends never move the prefix, so every rewrite must have LANDED. A
    // `Stale` here would mean the guard is refusing writes it should accept.
    let refused: Vec<&ReplaceOutcome> = report.outcomes.iter().filter(|o| !o.stored()).collect();
    assert!(
        refused.is_empty(),
        "an append-only race must never make a rewrite stale: {refused:?}"
    );
    let preserved: usize = report
        .outcomes
        .iter()
        .filter_map(|o| match o {
            ReplaceOutcome::ReplacedPreservingTail { preserved } => Some(*preserved),
            _ => None,
        })
        .sum();
    assert!(
        preserved > 0,
        "the tail-preservation path never fired, so the test did not actually race"
    );

    let final_conversation = stored(&sm, &id).await;
    let final_ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();

    // 1. Nothing appended was destroyed.
    let lost: Vec<&String> = appended
        .iter()
        .map(|(uid, _, _)| uid)
        .filter(|uid| !final_ids.contains(*uid))
        .collect();
    assert!(
        lost.is_empty(),
        "{} of {} appended messages were destroyed by racing rewrites (first few: {:?})",
        lost.len(),
        appended.len(),
        lost.iter().take(8).collect::<Vec<_>>()
    );

    // 3. msg_uid stability: the store accepted every id as given (no re-mint),
    // and each id still names the message it was created for.
    let reminted: Vec<&(String, String, String)> = appended
        .iter()
        .filter(|(uid, effective, _)| uid != effective)
        .collect();
    assert!(
        reminted.is_empty(),
        "unique ids must not be re-minted under concurrency: {reminted:?}"
    );
    let by_id: HashMap<String, String> = final_conversation
        .messages()
        .iter()
        .filter_map(|m| m.id.clone().map(|id| (id, text_of(m))))
        .collect();
    for (uid, _, text) in &appended {
        assert_eq!(
            by_id.get(uid).map(String::as_str),
            Some(text.as_str()),
            "msg_uid {uid} no longer names its own message"
        );
    }

    // 2. Nothing was duplicated.
    assert_no_duplicates(&final_conversation, "final");
    assert_eq!(
        final_conversation.len(),
        1 + appended.len() + REWRITES,
        "the final history must be seed + every acknowledged append + every summary"
    );

    // Write-lock starvation is tolerated (see `is_busy`) but must stay a small
    // minority; a real locking regression shows up as a burst. Measured on this
    // workload: 1-3 of 300 idle, up to 8 of 300 with the machine saturated, so
    // a quarter is a wide margin over observed noise that still catches a
    // rewrite that stopped releasing the lock.
    let busy = busy.lock().unwrap();
    let attempted = APPENDERS * PER_APPENDER;
    assert!(
        busy.len() * 4 < attempted,
        "{} of {attempted} appends failed with SQLITE_BUSY (>25%): the rewrite \
         transaction is holding the write lock far longer than measured",
        busy.len()
    );
    eprintln!(
        "tolerated: {} of {attempted} appends hit the busy timeout",
        busy.len()
    );

    // 7. No reader ever saw a partial rewrite.
    assert!(
        torn.lock().unwrap().is_empty(),
        "torn read observed: {:?}",
        torn.lock().unwrap()
    );
    assert!(reads.load(Ordering::Relaxed) > 0, "the reader never ran");
}

// ── 2. destructive (real compaction) rewrites + FTS ──────────────────────────

/// The same storm, but the rewriters do what compaction actually does: throw
/// away the summarized prefix. A stored message may then legitimately vanish —
/// but ONLY if some rewriter deliberately dropped it. Anything else is loss.
///
/// Also asserts the FTS recall mirror tracks the rewrite in both directions:
/// nothing that survived went missing from the index, and nothing that was
/// compacted away is still findable.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn compaction_shaped_rewrites_only_drop_what_they_meant_to() {
    const APPENDERS: usize = 5;
    const PER_APPENDER: usize = 40;
    const REWRITES: usize = 40;
    const KEEP_TAIL: usize = 4;

    let temp = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let id = seeded_session(&sm, "stress-2").await;

    let mut appenders = Vec::new();
    for a in 0..APPENDERS {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        appenders.push(tokio::spawn(async move {
            let mut written = Vec::new();
            for i in 0..PER_APPENDER {
                let uid = format!("note-{a}-{i}");
                // A distinctive token per message so FTS recall can be checked
                // one message at a time. FIXED WIDTH, deliberately: recall
                // builds a PREFIX query (`sanitize_fts_query` emits `"tok"*`),
                // so `zqx3v3` would also match `zqx3v33` and the "this message
                // left the index" half of the assertion below would be a lie.
                // Equal-length distinct tokens cannot prefix one another.
                let text = format!("zqx{a}v{i:03} appended note");
                match sm.add_message(&id, &user(&uid, &text)).await {
                    Ok(_) => written.push((uid, text)),
                    Err(e) if is_busy(&e.to_string()) => {}
                    Err(e) => panic!("append {uid} failed for a non-lock reason: {e}"),
                }
                tokio::task::yield_now().await;
            }
            written
        }));
    }

    let mut rewriters = Vec::new();
    for r in 0..2 {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        rewriters.push(tokio::spawn(async move {
            let mut report = RewriteReport {
                outcomes: Vec::new(),
                errors: Vec::new(),
                dropped: HashSet::new(),
            };
            for i in 0..REWRITES {
                let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
                let known = session.conversation.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;

                let messages = known.messages();
                let cut = messages.len().saturating_sub(KEEP_TAIL);
                let mut replacement = vec![assistant(
                    &format!("sum-{r}-{i}"),
                    &format!("compaction summary {r}/{i}"),
                )];
                replacement.extend_from_slice(&messages[cut..]);

                match sm
                    .replace_conversation_preserving_tail(
                        &id,
                        &Conversation::new_unvalidated(replacement),
                        basis,
                        &known,
                    )
                    .await
                {
                    Ok((outcome, _)) => {
                        if outcome.stored() {
                            // Only a rewrite that LANDED actually dropped
                            // anything.
                            report
                                .dropped
                                .extend(messages[..cut].iter().filter_map(|m| m.id.clone()));
                        }
                        report.outcomes.push(outcome);
                    }
                    Err(e) => report.errors.push(e.to_string()),
                }
            }
            report
        }));
    }

    let mut appended: Vec<(String, String)> = Vec::new();
    for handle in appenders {
        appended.extend(handle.await.unwrap());
    }
    let mut errors = Vec::new();
    let mut dropped = HashSet::new();
    let mut outcomes = Vec::new();
    for handle in rewriters {
        let report = handle.await.unwrap();
        errors.extend(report.errors);
        dropped.extend(report.dropped);
        outcomes.extend(report.outcomes);
    }

    assert!(errors.is_empty(), "rewrites must not error: {errors:?}");
    let landed = outcomes.iter().filter(|o| o.stored()).count();
    assert!(
        landed > 0,
        "no rewrite landed; the test proved nothing ({:?})",
        &outcomes[..outcomes.len().min(8)]
    );

    let final_conversation = stored(&sm, &id).await;
    assert_no_duplicates(&final_conversation, "final");
    let final_ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();

    let lost: Vec<&String> = appended
        .iter()
        .map(|(uid, _)| uid)
        .filter(|uid| !final_ids.contains(*uid) && !dropped.contains(*uid))
        .collect();
    assert!(
        lost.is_empty(),
        "{} of {} appended messages vanished without any rewriter dropping them \
         (first few: {:?})",
        lost.len(),
        appended.len(),
        lost.iter().take(8).collect::<Vec<_>>()
    );

    // 5. FTS mirror, both directions.
    for (uid, text) in &appended {
        let token = text.split_whitespace().next().unwrap();
        let hits = sm
            .search_chat_history(
                token,
                Some(20),
                None,
                None,
                None,
                // Full reach: this asserts the FTS mirror's contents, not #56's
                // visibility filter, and the fixture's sessions are public.
                biorouter::session::chat_history_search::SearchReach::tier_only(
                    biorouter::privacy::ProviderTier::Private,
                ),
            )
            .await
            .unwrap();
        let found = !hits.results.is_empty();
        if final_ids.contains(uid) {
            assert!(
                found,
                "surviving message {uid} ({token}) fell out of the recall index"
            );
        } else {
            assert!(
                !found,
                "compacted-away message {uid} ({token}) is still in the recall index"
            );
        }
    }

    // And the mirror holds exactly one row per indexed surviving message.
    let pool = probe(&temp).await;
    let fts_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts WHERE session_id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let indexable = final_conversation
        .messages()
        .iter()
        .filter(|m| m.metadata.user_visible && !text_of(m).is_empty())
        .count() as i64;
    assert_eq!(
        fts_rows, indexable,
        "the FTS mirror drifted from the stored message set"
    );
    pool.close().await;
}

// ── 3. cross-pool: two processes on one sessions.db ──────────────────────────

/// The CLI-vs-daemon shape at pressure: four INDEPENDENT stores appending
/// (their own pools, their own WAL readers, nothing in process memory ordering
/// them) while a fifth store rewrites. Only the guard inside the rewrite
/// transaction can save these appends.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn cross_pool_appends_survive_a_rewriting_daemon() {
    const APPENDERS: usize = 4;
    const PER_APPENDER: usize = 40;
    const REWRITES: usize = 40;

    let temp = TempDir::new().unwrap();
    let daemon = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let id = seeded_session(&daemon, "stress-3").await;

    let mut appenders = Vec::new();
    for a in 0..APPENDERS {
        // A brand-new manager per task: its own connection pool over the same
        // file, exactly like a separate `biorouter` process.
        let cli = SessionManager::new(temp.path().to_path_buf());
        let id = id.clone();
        appenders.push(tokio::spawn(async move {
            let mut written = Vec::new();
            for i in 0..PER_APPENDER {
                let uid = format!("cli-{a}-{i}");
                match cli
                    .add_message(&id, &user(&uid, &format!("term log {a}/{i}")))
                    .await
                {
                    Ok(_) => written.push(uid),
                    Err(e) if is_busy(&e.to_string()) => {}
                    Err(e) => panic!("cross-pool append {uid} failed: {e}"),
                }
                tokio::task::yield_now().await;
            }
            written
        }));
    }

    let rewriter = {
        let daemon = Arc::clone(&daemon);
        let id = id.clone();
        tokio::spawn(async move {
            let mut errors = Vec::new();
            let mut outcomes = Vec::new();
            for i in 0..REWRITES {
                let (session, basis) = daemon.snapshot_for_rewrite(&id).await.unwrap();
                let known = session.conversation.unwrap();
                tokio::time::sleep(Duration::from_millis(2)).await;
                let mut msgs = known.messages().clone();
                msgs.push(assistant(&format!("d-sum-{i}"), &format!("summary {i}")));
                match daemon
                    .replace_conversation_preserving_tail(
                        &id,
                        &Conversation::new_unvalidated(msgs),
                        basis,
                        &known,
                    )
                    .await
                {
                    Ok((outcome, _)) => outcomes.push(outcome),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            (outcomes, errors)
        })
    };

    let mut appended: Vec<String> = Vec::new();
    for handle in appenders {
        appended.extend(handle.await.unwrap());
    }
    let (outcomes, errors) = rewriter.await.unwrap();
    assert!(
        errors.is_empty(),
        "cross-pool rewrites must not fail: {errors:?}"
    );
    let refused: Vec<&ReplaceOutcome> = outcomes.iter().filter(|o| !o.stored()).collect();
    assert!(
        refused.is_empty(),
        "cross-pool appends must not make a rewrite stale: {refused:?}"
    );

    let final_conversation = stored(&daemon, &id).await;
    assert_no_duplicates(&final_conversation, "cross-pool final");
    let final_ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();
    let lost: Vec<&String> = appended
        .iter()
        .filter(|u| !final_ids.contains(*u))
        .collect();
    assert!(
        lost.is_empty(),
        "{} of {} messages appended by another process were destroyed (first few: {:?})",
        lost.len(),
        appended.len(),
        lost.iter().take(8).collect::<Vec<_>>()
    );
    assert_eq!(final_conversation.len(), 1 + appended.len() + REWRITES);
}

// ── 4. BR-7 blobs recovered on the tail ──────────────────────────────────────

/// An oversized tool response appended into the rewrite window is recovered as
/// a STUB (`scan_foreign_tail` deliberately does not hydrate). Its payload must
/// therefore survive the rewrite's orphan sweep, still hydrate on read, and not
/// be duplicated in `message_blobs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovered_tail_blobs_survive_the_sweep() {
    const BLOBS: usize = 12;
    const REWRITES: usize = 20;

    let temp = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let id = seeded_session(&sm, "stress-4").await;

    let appender = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        tokio::spawn(async move {
            let mut written = Vec::new();
            for i in 0..BLOBS {
                let uid = format!("blob-{i}");
                let marker = format!("payload{i}");
                sm.add_message(&id, &blob_message(&uid, &marker))
                    .await
                    .unwrap();
                written.push((uid, marker));
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            written
        })
    };
    let rewriter = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        tokio::spawn(async move {
            let mut errors = Vec::new();
            for i in 0..REWRITES {
                let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
                let known = session.conversation.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
                let mut msgs = known.messages().clone();
                msgs.push(assistant(&format!("b-sum-{i}"), &format!("summary {i}")));
                if let Err(e) = sm
                    .replace_conversation_preserving_tail(
                        &id,
                        &Conversation::new_unvalidated(msgs),
                        basis,
                        &known,
                    )
                    .await
                {
                    errors.push(e.to_string());
                }
            }
            errors
        })
    };
    let (written, errors) = tokio::join!(appender, rewriter);
    let written = written.unwrap();
    assert!(errors.unwrap().is_empty(), "blob rewrites must not error");

    let final_conversation = stored(&sm, &id).await;
    assert_no_duplicates(&final_conversation, "blob final");
    let by_id: HashMap<String, String> = final_conversation
        .messages()
        .iter()
        .filter_map(|m| m.id.clone().map(|id| (id, text_of(m))))
        .collect();

    for (uid, marker) in &written {
        let text = by_id
            .get(uid)
            .unwrap_or_else(|| panic!("blob message {uid} was destroyed by a rewrite"));
        assert!(
            text.contains(&format!("{marker} row 2999 ")),
            "blob message {uid} did not hydrate to its full payload \
             (len {}, starts {:?})",
            text.len(),
            text.chars().take(60).collect::<String>()
        );
    }

    // One row per payload, none orphaned.
    let pool = probe(&temp).await;
    let blob_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM message_blobs WHERE session_id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        blob_rows, BLOBS as i64,
        "message_blobs must hold exactly one row per stored payload"
    );
    pool.close().await;
}

// ── 5. a truncating writer racing the rewriters ──────────────────────────────

/// A checkpoint restore / message edit (`truncate_conversation`) moves the
/// basis' prefix, which the guard must refuse outright rather than merge onto.
/// The invariant is that a refused rewrite writes NOTHING: it may not resurrect
/// truncated messages, and it may not lose messages appended after the
/// truncate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_racing_truncate_is_refused_and_never_resurrects() {
    let temp = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let id = seeded_session(&sm, "stress-5").await;

    // Everything before the cutoff is truncatable; everything after is not.
    for i in 0..20 {
        sm.add_message(
            &id,
            &Message::new(
                rmcp::model::Role::User,
                1_000 + i as i64,
                vec![MessageContent::text(format!("old {i}"))],
            )
            .with_id(format!("old-{i}")),
        )
        .await
        .unwrap();
    }

    // The FIRST rewrite is handshaken so the refusal is guaranteed, not hoped
    // for: the rewriter parks after taking its snapshot, the truncator moves
    // the prefix, and only then does the rewrite go in. Left to chance, the
    // truncate has to land inside a millisecond-wide window, and on a loaded
    // machine (measured: 1 failure in 12 runs under 10 CPU burners) a whole run
    // sees zero overlap and the assertion fails with no bug present. Every
    // rewrite after the first is unsynchronized, so the stress is unchanged.
    let snapshot_taken = Arc::new(AtomicBool::new(false));
    let prefix_moved = Arc::new(AtomicBool::new(false));
    let rewriting = Arc::new(AtomicBool::new(true));

    let rewriter = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        let rewriting = Arc::clone(&rewriting);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let prefix_moved = Arc::clone(&prefix_moved);
        tokio::spawn(async move {
            let mut outcomes = Vec::new();
            let mut errors = Vec::new();
            for i in 0..30 {
                let (session, basis) = sm.snapshot_for_rewrite(&id).await.unwrap();
                let known = session.conversation.unwrap();
                if i == 0 {
                    snapshot_taken.store(true, Ordering::SeqCst);
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    while !prefix_moved.load(Ordering::SeqCst) {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "the truncator never moved the prefix; the handshake is broken"
                        );
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                } else {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                let mut msgs = known.messages().clone();
                msgs.push(assistant(&format!("t-sum-{i}"), &format!("summary {i}")));
                match sm
                    .replace_conversation_preserving_tail(
                        &id,
                        &Conversation::new_unvalidated(msgs),
                        basis,
                        &known,
                    )
                    .await
                {
                    Ok((outcome, _)) => outcomes.push(outcome),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            rewriting.store(false, Ordering::SeqCst);
            (outcomes, errors)
        })
    };
    let truncator = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        let rewriting = Arc::clone(&rewriting);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let prefix_moved = Arc::clone(&prefix_moved);
        tokio::spawn(async move {
            while !snapshot_taken.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            // Drop everything at or after `old-10`, under the parked snapshot.
            sm.truncate_conversation(&id, 1_010).await.unwrap();
            prefix_moved.store(true, Ordering::SeqCst);

            // Then keep truncating, unsynchronized, for the whole rewriter
            // lifetime.
            let mut i = 0;
            while rewriting.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(2)).await;
                sm.truncate_conversation(&id, 1_010).await.unwrap();
                match sm
                    .add_message(&id, &user(&format!("after-{i}"), &format!("after {i}")))
                    .await
                {
                    Ok(_) => {}
                    Err(e) if is_busy(&e.to_string()) => {}
                    Err(e) => panic!("truncator append failed: {e}"),
                }
                i += 1;
            }
        })
    };
    let (rewrites, truncs) = tokio::join!(rewriter, truncator);
    truncs.unwrap();
    let (outcomes, errors) = rewrites.unwrap();
    assert!(errors.is_empty(), "rewrites must not error: {errors:?}");
    assert_eq!(
        outcomes.first(),
        Some(&ReplaceOutcome::Stale),
        "a truncate that lands under a parked snapshot MUST be refused; got {outcomes:?}"
    );

    // Final truncate, so the end state is unambiguous.
    sm.truncate_conversation(&id, 1_010).await.unwrap();
    let final_conversation = stored(&sm, &id).await;
    assert_no_duplicates(&final_conversation, "truncate final");
    let ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();
    for i in 10..20 {
        assert!(
            !ids.contains(&format!("old-{i}")),
            "a refused rewrite resurrected truncated message old-{i}"
        );
    }
    for i in 0..10 {
        assert!(
            ids.contains(&format!("old-{i}")),
            "message old-{i} was below the truncate cutoff and must survive"
        );
    }
}

// ── 6. the unguarded exception is still unguarded, on purpose ────────────────

/// The control. `replace_conversation` is the NAMED EXCEPTION — it is still an
/// unconditional whole-history rewrite, and under the exact workload above it
/// still destroys concurrent appends. This test asserts that it does.
///
/// Two things depend on it. It proves the harness's loss detector actually
/// detects loss (a race test that passes on broken code is worthless), and it
/// pins the semantics of the exception, so a future change that quietly makes
/// `replace_conversation` guard itself has to come here and say so.
///
/// **The loss is handshaken, not hoped for.** Left to the scheduler this test
/// asserts that a race it does not control happened to occur: a run that
/// serialized the appender ahead of the first rewrite would destroy nothing and
/// fail with no bug present and no behaviour changed — the single worst failure
/// mode a control can have, because it teaches the reader to distrust it. So the
/// first rewrite parks after its snapshot until one append has COMMITTED, which
/// makes `ctl-0` a message the rewrite provably never saw and provably deletes.
/// Every rewrite after the first is unsynchronized, so the storm is unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_unguarded_rewrite_still_destroys_concurrent_appends() {
    const APPENDS: usize = 300;
    const REWRITES: usize = 60;
    /// The append the handshake guarantees lands strictly between the first
    /// rewrite's snapshot and its write-back.
    const VICTIM: &str = "ctl-0";

    let temp = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let id = seeded_session(&sm, "control").await;

    let snapshot_taken = Arc::new(AtomicBool::new(false));
    let victim_committed = Arc::new(AtomicBool::new(false));

    let appender = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let victim_committed = Arc::clone(&victim_committed);
        tokio::spawn(async move {
            // Half one of the handshake: not a single append until the rewriter
            // holds a snapshot, so `ctl-0` cannot possibly be inside it.
            while !snapshot_taken.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let mut written = Vec::new();
            for i in 0..APPENDS {
                let uid = format!("ctl-{i}");
                match sm.add_message(&id, &user(&uid, &format!("note {i}"))).await {
                    Ok(_) => written.push(uid),
                    // Tolerated for the same reason as everywhere else in this
                    // file (see `is_busy`) — the caller was TOLD it failed, so
                    // it is not loss. `ctl-0` cannot land here: the rewriter is
                    // parked and holding no lock when it is written.
                    Err(e) if is_busy(&e.to_string()) => {
                        assert_ne!(uid, VICTIM, "the handshaken append must not be starved");
                    }
                    Err(e) => panic!("control append {uid} failed for a non-lock reason: {e}"),
                }
                if i == 0 {
                    victim_committed.store(true, Ordering::SeqCst);
                }
                tokio::task::yield_now().await;
            }
            written
        })
    };
    let rewriter = {
        let sm = Arc::clone(&sm);
        let id = id.clone();
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let victim_committed = Arc::clone(&victim_committed);
        tokio::spawn(async move {
            for i in 0..REWRITES {
                // Exactly what every in-turn compaction site did before the fix:
                // read a snapshot, take time, write the whole thing back.
                let known = stored(&sm, &id).await;
                if i == 0 {
                    // Half two: announce the snapshot, then park until the
                    // append it cannot contain is durable.
                    snapshot_taken.store(true, Ordering::SeqCst);
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    while !victim_committed.load(Ordering::SeqCst) {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "the appender never committed; the handshake is broken"
                        );
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    assert!(
                        !ids_of(&known).iter().any(|u| u == VICTIM),
                        "the parked snapshot must predate the victim append"
                    );
                } else {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                let mut msgs = known.messages().clone();
                msgs.push(assistant(&format!("c-sum-{i}"), &format!("summary {i}")));
                sm.replace_conversation(&id, &Conversation::new_unvalidated(msgs))
                    .await
                    .unwrap();
            }
        })
    };
    let appended = appender.await.unwrap();
    rewriter.await.unwrap();

    let final_ids: HashSet<String> = ids_of(&stored(&sm, &id).await).into_iter().collect();
    let lost: Vec<&String> = appended
        .iter()
        .filter(|u| !final_ids.contains(*u))
        .collect();
    assert!(
        appended.iter().any(|u| u == VICTIM),
        "the store acknowledged the handshaken append, or there is nothing to lose"
    );
    assert!(
        lost.iter().any(|u| *u == VICTIM),
        "the unguarded rewrite kept an append it demonstrably never read, so it has \
         grown a guard it must not have, and every other test in this file is now \
         asserting a property nothing is testing"
    );
    assert!(
        !lost.is_empty(),
        "the unguarded rewrite destroyed nothing under a 300-append storm; either \
         the workload stopped racing (in which case every other test in this file \
         is now vacuous) or `replace_conversation` grew a guard it must not have"
    );
    eprintln!(
        "control: the unguarded rewrite destroyed {} of {} appends",
        lost.len(),
        appended.len()
    );
}

// ── 7. end to end: a real turn auto-compacting under an append storm ─────────

/// A summary that clears `summary_is_usable`, so compaction accepts it first
/// time.
const GOOD_SUMMARY: &str = "## User Intent\nThe user asked for a plot.\n\
     ## Technical Concepts\nPlotting, data frames.\n\
     ## Files\nnone\n## Pending Tasks\nnone\n";

/// A provider whose summarization round-trip takes real wall-clock time and
/// announces itself, so appenders can be aimed precisely at the window between
/// the compaction's snapshot and its write-back. Everything appended while
/// `summarizing` is true landed strictly AFTER the snapshot, so compaction has
/// never seen it and may not drop it.
struct SlowSummarizer {
    summarizing: Arc<AtomicBool>,
    summarizer_calls: AtomicUsize,
    /// Completions that are the reply loop's own, not the summarizer's.
    main_calls: AtomicUsize,
    /// Zero-based main-call indices that answer `ContextLengthExceeded`, which
    /// drives the loop's own overflow-recovery compaction rather than the
    /// auto-compaction in `reply()`.
    overflow_on: Vec<usize>,
    window: Duration,
    context_limit: usize,
    session_id: OnceLock<String>,
    /// Every system prompt a summarization round-trip was handed. Compaction
    /// carries the history it is condensing in there, so this is the only way a
    /// test outside the loop can tell "a compaction deliberately summarized this
    /// message away" from "something destroyed it".
    payloads: Mutex<Vec<String>>,
}

impl SlowSummarizer {
    fn is_summarizer_call(messages: &[Message]) -> bool {
        messages.len() == 1
            && messages[0].content.iter().any(|c| {
                matches!(c, MessageContent::Text(t)
                    if t.text.contains("Please summarize the conversation history"))
            })
    }
}

#[async_trait]
impl Provider for SlowSummarizer {
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if Self::is_summarizer_call(messages) {
            self.payloads
                .lock()
                .unwrap()
                .push(_system_prompt.to_string());
            self.summarizer_calls.fetch_add(1, Ordering::SeqCst);
            self.summarizing.store(true, Ordering::SeqCst);
            tokio::time::sleep(self.window).await;
            self.summarizing.store(false, Ordering::SeqCst);
            return Ok((Message::assistant().with_text(GOOD_SUMMARY), usage));
        }
        let n = self.main_calls.fetch_add(1, Ordering::SeqCst);
        if self.overflow_on.contains(&n) {
            return Err(ProviderError::ContextLengthExceeded(
                "mock overflow".to_string(),
            ));
        }
        Ok((Message::assistant().with_text("All done."), usage))
    }

    fn get_model_config(&self) -> ModelConfig {
        let mut config = ModelConfig::new("mock-model").unwrap();
        config.context_limit = Some(self.context_limit);
        config
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for testing".to_string(),
            default_model: "mock-model".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "mock-stress"
    }
}

/// The BR-71 scenario at pressure, through the REAL reply loop: a turn is
/// running and hits auto-compaction; while its summarizer works, four other
/// writers append to the same session — a note tool, `biorouter term log`,
/// another socket. Every one of those appends was acknowledged before the
/// compaction wrote back, so every one of them must be on disk when the turn
/// ends.
///
/// Only appends made INSIDE the summarization window count: a message that
/// landed before the snapshot is part of what compaction is entitled to
/// summarize away.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_live_turn_compacting_cannot_lose_concurrent_appends() {
    const APPENDERS: usize = 4;

    let work_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));

    let summarizing = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(SlowSummarizer {
        summarizing: Arc::clone(&summarizing),
        summarizer_calls: AtomicUsize::new(0),
        main_calls: AtomicUsize::new(0),
        overflow_on: Vec::new(),
        window: Duration::from_millis(400),
        context_limit: 100,
        session_id: OnceLock::new(),
        payloads: Mutex::new(Vec::new()),
    });

    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        Arc::clone(&sm),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    )));
    let session = sm
        .create_session(
            work_dir.path().to_path_buf(),
            "stress-live-turn".into(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    provider.session_id.set(session.id.clone()).unwrap();
    agent
        .update_provider(Arc::clone(&provider) as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();

    // Enough history, and a live gauge close enough to the limit, that
    // `reply()` auto-compacts before its first model call.
    for i in 0..8 {
        sm.add_message(&session.id, &user(&format!("q{i}"), &format!("q{i}")))
            .await
            .unwrap();
        sm.add_message(&session.id, &assistant(&format!("a{i}"), &format!("a{i}")))
            .await
            .unwrap();
    }
    sm.update(&session.id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    let turn_done = Arc::new(AtomicBool::new(false));
    let mut appenders = Vec::new();
    for a in 0..APPENDERS {
        let sm = Arc::clone(&sm);
        let id = session.id.clone();
        let summarizing = Arc::clone(&summarizing);
        let turn_done = Arc::clone(&turn_done);
        appenders.push(tokio::spawn(async move {
            // Wait for the compaction's snapshot to be taken and the
            // summarizer to be in flight.
            while !summarizing.load(Ordering::SeqCst) && !turn_done.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let mut written = Vec::new();
            let mut i = 0;
            while summarizing.load(Ordering::SeqCst) {
                let uid = format!("live-{a}-{i}");
                match sm
                    .add_message(
                        &id,
                        &user(&uid, &format!("NOTE {a}/{i} from another writer")),
                    )
                    .await
                {
                    Ok(_) => written.push(uid),
                    Err(e) if is_busy(&e.to_string()) => {}
                    Err(e) => panic!("append {uid} failed for a non-lock reason: {e}"),
                }
                i += 1;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            written
        }));
    }

    let session_config = SessionConfig {
        id: session.id.clone(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(
            Message::user().with_text("Plot the data"),
            session_config,
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    turn_done.store(true, Ordering::SeqCst);

    let mut appended: Vec<String> = Vec::new();
    for handle in appenders {
        appended.extend(handle.await.unwrap());
    }

    assert_eq!(
        provider.summarizer_calls.load(Ordering::SeqCst),
        1,
        "the turn must have auto-compacted exactly once, or the test proved nothing"
    );
    assert!(
        appended.len() >= APPENDERS,
        "too few appends landed inside the summarization window ({}); the test \
         did not actually race",
        appended.len()
    );

    let final_conversation = stored(&sm, &session.id).await;
    assert_no_duplicates(&final_conversation, "live turn");
    let final_ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();
    let lost: Vec<&String> = appended
        .iter()
        .filter(|u| !final_ids.contains(*u))
        .collect();
    assert!(
        lost.is_empty(),
        "{} of {} messages appended DURING the turn's compaction were destroyed \
         by its write-back (first few: {:?})",
        lost.len(),
        appended.len(),
        lost.iter().take(8).collect::<Vec<_>>()
    );
    eprintln!(
        "live turn: {} concurrent appends landed inside the summarization window, \
         all {} survived",
        appended.len(),
        appended.len()
    );

    drop(work_dir);
    drop(data_dir);
}

// ── 8. ABA: a session id freed by a wipe is handed straight back out ──────────

/// `create_session` allocates `YYYYMMDD_N` from `MAX(...) + 1` over the
/// `sessions` table, so emptying that table restarts the counter and the next
/// session is handed the id the old one had. A rewrite that took its basis
/// before the wipe is then pointed at a *different conversation under the same
/// name* — and the guard it relies on compares counts and rowids, which say
/// nothing about identity.
///
/// This is reachable from the app: Settings -> Reset -> History wipes every
/// session while a turn (or a background eager compaction) is mid-flight. The
/// rewrite must be refused; accepting it deletes messages the user wrote after
/// the reset and replaces them with a summary of a conversation that no longer
/// exists.
///
/// Measured scope, so nobody over-reads it: this is the end-to-end shape over a
/// real on-disk store, and it discriminates the half of the fix that stopped the
/// wipe rewinding `sqlite_sequence` — with that restored it accepts the stale
/// rewrite (`Replaced`, not `Stale`). With the rowids left climbing it passes
/// even if the per-row incarnation token is neutered, because the prefix check
/// alone then refuses. The token's independent proof is
/// `session_manager::tests::a_guarded_rewrite_is_refused_even_when_the_rowids_are_replayed_exactly`,
/// which replays the old rowids one for one so only row identity can separate
/// the incarnations.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rewrite_basis_cannot_cross_a_wipe_that_recycled_the_session_id() {
    let temp = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));

    let first = sm
        .create_session(
            PathBuf::from("/tmp/stress"),
            "aba-first-incarnation".into(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;
    sm.add_message(&first, &user("old-1", "a message from the old session"))
        .await
        .unwrap();

    // A compaction (or any rewrite) takes its basis and goes off to summarize.
    let (session, basis) = sm.snapshot_for_rewrite(&first).await.unwrap();
    let known = session.conversation.unwrap();

    // ...and while it is away, the user resets their history and starts again.
    sm.clear_all_sessions().await.unwrap();
    // A reset does not lower `session_id_high_water`, so this build alone gives
    // the new chat a fresh id and the ABA never arises. The rewrite guard still
    // has to close it on its own, because a build without the mark sharing the
    // file — or a restored backup — can hand the id back; forcing the reuse is
    // reproducing exactly what those leave behind.
    sm.forget_minted_session_ids_for_test().await.unwrap();
    let second = sm
        .create_session(
            PathBuf::from("/tmp/stress"),
            "aba-second-incarnation".into(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;
    assert_eq!(
        second, first,
        "the wipe must hand the freed id back out, or this test proves nothing"
    );
    sm.add_message(&second, &user("new-1", "the new session's first message"))
        .await
        .unwrap();

    // The detached rewrite finally writes back, naming the id it started with.
    let mut replacement = known.messages().clone();
    replacement.push(assistant(
        "aba-sum",
        "summary of a conversation that no longer exists",
    ));
    let (outcome, _) = sm
        .replace_conversation_preserving_tail(
            &second,
            &Conversation::new_unvalidated(replacement),
            basis,
            &known,
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        ReplaceOutcome::Stale,
        "a basis from a previous incarnation of a recycled id must never be \
         accepted: same id and same rowids are not the same conversation"
    );

    let final_conversation = stored(&sm, &second).await;
    let ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();
    assert!(
        ids.contains("new-1"),
        "the message the user wrote after the reset must survive; got {:?}",
        ids_of(&final_conversation)
    );
    assert!(
        !ids.contains("aba-sum"),
        "a refused rewrite must write nothing at all"
    );
    assert!(
        !ids.contains("old-1"),
        "and it must not resurrect the wiped session's messages into the new one"
    );
}

// ── 9. window-agnostic: a real turn, two overflow recoveries, one hammering ──
//      appender

/// The net that does not need to know where the windows are.
///
/// `conversation_writeback_freshness.rs` aims a single append at one named
/// instant, which is the only way to make a specific defect fail on demand — but
/// it can only cover instants somebody already thought of. Every window in this
/// area so far was one nobody had: the split between `reply()`'s snapshot and
/// the loop's basis read, and the split between a landed swap and its basis
/// refresh. Both sat outside every provider call, so the entire pre-existing
/// suite was structurally unable to reach them.
///
/// So: run three real turns back to back, each rewriting the history at least
/// twice (two overflow recoveries, plus `reply()`'s auto-compaction whenever the
/// gauge is over), with an outside writer appending as fast as the store will
/// take it from before the first turn starts until after the last one ends. That
/// covers every instant of every turn, including the ones this file has not
/// thought of yet.
///
/// **"Every acknowledged append survives" is the WRONG invariant here**, and the
/// first draft of this test asserted it and failed on correct code: with three
/// compactions in one turn, an append that lands before a later compaction's
/// snapshot is inside that compaction's `known` and gets summarized away, which
/// is the feature working. Measured on the first run: 23 of 73 appends missing,
/// all 23 handed to a summarizer, none destroyed.
///
/// The precise invariant — the same one
/// `compaction_shaped_rewrites_only_drop_what_they_meant_to` uses — is that a
/// message may only be missing if some compaction was actually SHOWN it.
/// Compaction carries the history it is condensing in the summarizer's system
/// prompt, so recording those prompts tells us exactly which messages a
/// compaction was entitled to drop. Everything else is owed.
///
/// Residual, stated rather than hidden: a message that was shown to a
/// summarization whose swap was then DECLINED, and destroyed by a later one,
/// would be excused. Every swap lands in this test, and the aimed tests in
/// `conversation_writeback_freshness.rs` cover the declined path directly.
///
/// It is a net, not a proof: a run in which no append happens to land inside a
/// window passes vacuously. That is why the aimed tests exist as well — and why
/// this one asserts a floor on how many appends were genuinely owed.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_live_turn_recovering_from_two_overflows_cannot_lose_concurrent_appends() {
    /// How many turns to run back to back. Each opens its own set of windows.
    const TURNS: usize = 3;

    let work_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));

    let summarizing = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(SlowSummarizer {
        summarizing: Arc::clone(&summarizing),
        summarizer_calls: AtomicUsize::new(0),
        main_calls: AtomicUsize::new(0),
        // Two overflows per turn, over TURNS turns: each turn's model calls are
        // `3t`, `3t+1` (both overflow, each driving a recovery compaction and a
        // write-back) and `3t+2` (succeeds and ends the turn). Windows this
        // narrow are hit probabilistically, so the test opens several.
        overflow_on: (0..TURNS).flat_map(|t| [3 * t, 3 * t + 1]).collect(),
        window: Duration::from_millis(30),
        context_limit: 100,
        session_id: OnceLock::new(),
        payloads: Mutex::new(Vec::new()),
    });

    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        Arc::clone(&sm),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    )));
    let session = sm
        .create_session(
            work_dir.path().to_path_buf(),
            "stress-two-overflows".into(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    provider.session_id.set(session.id.clone()).unwrap();
    agent
        .update_provider(Arc::clone(&provider) as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();

    for i in 0..8 {
        sm.add_message(&session.id, &user(&format!("q{i}"), &format!("q{i}")))
            .await
            .unwrap();
        sm.add_message(&session.id, &assistant(&format!("a{i}"), &format!("a{i}")))
            .await
            .unwrap();
    }
    sm.update(&session.id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    // The outside writer: BR-71's note tool, `biorouter term log` from a shell
    // hook, a second socket. It starts before the turn and runs until it ends,
    // so no instant of the turn is uncovered.
    let turn_done = Arc::new(AtomicBool::new(false));
    let appender = {
        let sm = Arc::clone(&sm);
        let id = session.id.clone();
        let turn_done = Arc::clone(&turn_done);
        let summarizing = Arc::clone(&summarizing);
        tokio::spawn(async move {
            let mut written = Vec::new();
            let mut i = 0usize;
            while !turn_done.load(Ordering::SeqCst) {
                let uid = format!("hammer-{i}");
                // FIXED WIDTH: the marker is matched against the summarizer's
                // payload below, and `NOTE-7 ` would also match inside
                // `NOTE-70 `, silently excusing a destroyed message.
                let text = format!("NOTE-{i:04} from another writer");
                let before = summarizing.load(Ordering::SeqCst);
                match sm.add_message(&id, &user(&uid, &text)).await {
                    // Acknowledged. Owed unless a compaction is shown it.
                    // `raced` records that it landed while a summarization was
                    // in flight, i.e. strictly inside a snapshot -> write-back
                    // window: the file's standing measure of "this workload
                    // really did race" (see test 7).
                    Ok(_) => {
                        let raced = before || summarizing.load(Ordering::SeqCst);
                        written.push((uid, text, raced));
                    }
                    Err(e) if is_busy(&e.to_string()) => {}
                    Err(e) => panic!("append {uid} failed for a non-lock reason: {e}"),
                }
                i += 1;
                tokio::task::yield_now().await;
            }
            written
        })
    };

    for turn in 0..TURNS {
        let session_config = SessionConfig {
            id: session.id.clone(),
            schedule_id: None,
            max_turns: Some(6),
            max_tool_calls: None,
            retry_config: None,
            budget: None,
            reasoning_effort: None,
        };
        let stream = agent
            .reply(
                Message::user().with_text(format!("Plot the data, take {turn}")),
                session_config,
                None,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            event.unwrap();
        }
    }
    turn_done.store(true, Ordering::SeqCst);
    let appended = appender.await.unwrap();

    assert_eq!(
        provider.main_calls.load(Ordering::SeqCst),
        3 * TURNS,
        "each turn must be two overflows then a success"
    );
    assert!(
        provider.summarizer_calls.load(Ordering::SeqCst) >= 2 * TURNS,
        "every overflow must have run its own recovery compaction, saw {}",
        provider.summarizer_calls.load(Ordering::SeqCst)
    );

    // Non-vacuity, measured the way the rest of this file measures it: how many
    // acknowledged appends landed strictly inside a snapshot -> write-back
    // window. This is stable (a 30 ms summarization against sub-millisecond
    // appends) where a floor on `owed` below is not: with several compactions in
    // a row, most appends are legitimately summarized by a LATER one, so `owed`
    // varies with the schedule and a floor on it would flake.
    let raced = appended.iter().filter(|(_, _, raced)| *raced).count();
    assert!(
        raced >= 8,
        "only {raced} of {} appends landed while a summarization was in flight; \
         the writer never really raced the rewrites",
        appended.len()
    );

    // Everything a compaction actually SAW is fair game; everything else is owed.
    let shown_to_a_summarizer = provider.payloads.lock().unwrap().join("\n");
    let (summarizable, owed): (Vec<_>, Vec<_>) = appended
        .iter()
        .partition(|(_, text, _)| shown_to_a_summarizer.contains(text.as_str()));
    assert!(
        !owed.is_empty(),
        "every single append was handed to some compaction, so the invariant \
         below has nothing to assert"
    );

    let final_conversation = stored(&sm, &session.id).await;
    assert_no_duplicates(&final_conversation, "two-overflow turn");
    let final_ids: HashSet<String> = ids_of(&final_conversation).into_iter().collect();
    let lost: Vec<&String> = owed
        .iter()
        .map(|(uid, _, _)| uid)
        .filter(|uid| !final_ids.contains(*uid))
        .collect();
    assert!(
        lost.is_empty(),
        "{} of {} acknowledged appends were destroyed somewhere across three \
         history-rewriting turns, and NO compaction was ever shown them \
         (first few: {:?})",
        lost.len(),
        owed.len(),
        lost.iter().take(8).collect::<Vec<_>>()
    );
    eprintln!(
        "two-overflow turns: {} appends acknowledged, {raced} of them inside a \
         summarization window; {} owed and all survived, {} legitimately \
         summarized",
        appended.len(),
        owed.len(),
        summarizable.len()
    );

    drop(work_dir);
    drop(data_dir);
}
