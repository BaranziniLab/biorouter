//! The decision log end to end: the decision service writing through the
//! production sink into a real session store, read back by every reader a
//! surface uses, then gone with the chat. Offline: the model is an in-process
//! fake, and nothing reaches a network.
//!
//! The migration test opens a store the way an older build leaves it (schema
//! 20, no `decisions` table, chats and messages on disk) with raw SQL, then
//! lets `SessionManager` upgrade it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use biorouter::conversation::message::Message;
use biorouter::decision::log::{
    count_for_sessions, get, list, summary, DecisionQuery, SqliteDecisionSink,
};
use biorouter::decision::types::{
    DecisionAdapter, DecisionKindTag, ModelVerdicts, ProbabilitySource, QuestionVerdict,
};
use biorouter::decision::{
    DecisionCaps, DecisionChunk, DecisionContext, DecisionKind, DecisionModel, DecisionModelEntry,
    DecisionModelRef, DecisionModelSource, DecisionOrigin, DecisionOutcome, DecisionRequest,
    DecisionResult, DecisionService, DecisionSession, DecisionUseGate, Question, Threshold,
};
use biorouter::privacy::affiliation::ModelAffiliation;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::session::{SessionManager, SessionType};
use indexmap::IndexMap;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[ctor::ctor]
fn sandbox() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = tempfile::TempDir::new().expect("a sandbox");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

const MODEL: &str = "tev1:0.8b";

/// A local yes/no checker that says "yes" to any chunk mentioning MS, and
/// fails any chunk that says `FAIL`, so a batch has passes, fails and errors.
struct FakeChecker {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DecisionModel for FakeChecker {
    fn model_ref(&self) -> DecisionModelRef {
        DecisionModelRef::new("ollama_decision", MODEL).with_adapter(DecisionAdapter::SystemOne)
    }
    fn display_name(&self) -> String {
        MODEL.to_string()
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        None
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: vec![DecisionKindTag::Binary],
            max_questions: 64,
            context_tokens: 8_192,
            max_concurrency: 4,
            max_request_bytes: None,
            local: true,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = request.context.render_text();
        if text.contains("FAIL") {
            return Err(biorouter::decision::DecisionError::Invalid(
                "the model could not read this note".to_string(),
            ));
        }
        let p_yes = if text.contains("MS") { 0.96 } else { 0.08 };
        let verdicts = request
            .questions
            .keys()
            .map(|name| QuestionVerdict {
                question: name.clone(),
                probabilities: IndexMap::from([
                    ("yes".to_string(), p_yes),
                    ("no".to_string(), 1.0 - p_yes),
                ]),
                source: ProbabilitySource::Native,
                calibrated: true,
                reported_choice: None,
                raw: json!({"noul": p_yes}),
            })
            .collect();
        Ok(ModelVerdicts {
            verdicts,
            usage: Default::default(),
            served_model: Some(MODEL.to_string()),
        })
    }
}

struct OneModel {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DecisionModelSource for OneModel {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        vec![DecisionModelEntry {
            provider: "ollama_decision".to_string(),
            model: MODEL.to_string(),
            display_name: MODEL.to_string(),
            context_tokens: 8_192,
            kinds: vec![DecisionKindTag::Binary],
            tier: ProviderTier::Private,
            affiliation: None,
            adapter: DecisionAdapter::SystemOne,
            local: true,
        }]
    }
    async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        Ok(Arc::new(FakeChecker {
            calls: Arc::clone(&self.calls),
        }))
    }
}

/// No workspace in this test fences the chat.
struct NoCrew;

#[async_trait]
impl DecisionUseGate for NoCrew {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Ok(())
    }
}

fn has_ms() -> IndexMap<String, Question> {
    IndexMap::from([(
        "has_ms".to_string(),
        Question {
            instructions: "Does this note state the patient has multiple sclerosis?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.9)),
        },
    )])
}

async fn chat(manager: &SessionManager, dir: &tempfile::TempDir) -> String {
    let id = manager
        .create_session(
            dir.path().into(),
            "Synthetic notes".into(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;
    manager
        .add_message(&id, &Message::user().with_text("Find the MS patients."))
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn a_batch_is_one_row_per_chunk_and_every_reader_aggregates_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let manager = Arc::new(SessionManager::new(dir.path().into()));
    let id = chat(&manager, &dir).await;
    let quiet = chat(&manager, &dir).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let service = DecisionService::new(Arc::new(OneModel {
        calls: Arc::clone(&calls),
    }))
    .with_default_model(None)
    .with_use_gate(Arc::new(NoCrew))
    .with_sink(Arc::new(SqliteDecisionSink {
        session_manager: Arc::clone(&manager),
    }));

    let session = DecisionSession::new(&id, SessionClassification::Private, true)
        .with_origin(DecisionOrigin::Tool, Some("call_1".to_string()))
        .with_turn(Some("turn-1".to_string()))
        .with_origin_label(Some("decide".to_string()));
    let chunks: Vec<DecisionChunk> = (0..30)
        .map(|i| DecisionChunk {
            id: format!("note-{i:04}"),
            context: DecisionContext::Text(match i % 10 {
                0 => format!("SYNTHETIC NOTE {i}: FAIL"),
                1..=3 => format!("SYNTHETIC NOTE {i}: RRMS since 2019, MS clinic follow-up."),
                _ => format!("SYNTHETIC NOTE {i}: migraine, no demyelination."),
            }),
        })
        .collect();
    let outcomes = service
        .decide_batch(Some(&session), None, &has_ms(), chunks, "batch-1")
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 30);

    // A monitor in the same turn.
    let monitor = DecisionSession::new(&id, SessionClassification::Private, true)
        .with_origin(DecisionOrigin::Monitor, Some("form-reached".to_string()))
        .with_turn(Some("turn-1".to_string()))
        .with_origin_label(Some("Form reached".to_string()));
    service
        .decide(
            Some(&monitor),
            None,
            &DecisionRequest {
                questions: has_ms(),
                context: DecisionContext::Text("the agent is on the login page".into()),
            },
        )
        .await
        .unwrap();

    let batch = list(
        &manager,
        &id,
        &DecisionQuery {
            batch_id: Some("batch-1".to_string()),
            limit: Some(200),
            ..DecisionQuery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(batch.total, 30, "one row per chunk");
    assert_eq!(batch.records.len(), 30);
    let mut chunk_ids: Vec<_> = batch
        .records
        .iter()
        .map(|r| r.chunk_id.clone().unwrap())
        .collect();
    chunk_ids.sort();
    chunk_ids.dedup();
    assert_eq!(chunk_ids.len(), 30);
    let errors = batch
        .records
        .iter()
        .filter(|r| r.outcome == DecisionOutcome::Error)
        .count();
    assert_eq!(errors, 3);
    let failed_row = batch
        .records
        .iter()
        .find(|r| r.outcome == DecisionOutcome::Error)
        .unwrap();
    assert!(failed_row
        .error
        .as_deref()
        .unwrap()
        .contains("could not read"));
    assert!(failed_row.decisions.is_empty());

    let detail = get(&manager, &id, &batch.records[0].id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(detail.context, DecisionContext::Text(ref t) if t.starts_with("SYNTHETIC")));

    let summary = summary(&manager, &id).await.unwrap();
    assert_eq!(summary.total, 31);
    assert_eq!(
        summary.aggregates.len(),
        2,
        "one per source, never per call"
    );
    let tool = summary
        .aggregates
        .iter()
        .find(|a| a.origin == DecisionOrigin::Tool)
        .unwrap();
    assert_eq!(
        (tool.count, tool.passed, tool.failed, tool.errors),
        (30, 9, 18, 3)
    );
    assert_eq!(tool.label, "decide");
    assert_eq!(summary.recent_turns.len(), 2);
    assert!(summary.recent_turns.iter().all(|t| t.turn_id == "turn-1"));

    let counts = count_for_sessions(&manager, &[id.clone(), quiet.clone()])
        .await
        .unwrap();
    assert_eq!((counts[&id], counts[&quiet]), (31, 0));

    // History's per-chat count rides the full listing, and only where it is
    // not zero.
    let listed = manager.list_sessions().await.unwrap();
    let count_of = |wanted: &str| {
        listed
            .iter()
            .find(|s| s.id == wanted)
            .map(|s| s.decision_count)
            .unwrap()
    };
    assert_eq!(count_of(&id), Some(31));
    assert_eq!(count_of(&quiet), None);
    let wire = serde_json::to_value(listed.iter().find(|s| s.id == quiet).unwrap()).unwrap();
    assert!(wire.get("decision_count").is_none(), "absent, not 0");

    // Deleting the chat takes every row with it.
    manager.delete_session(&id).await.unwrap();
    assert_eq!(
        list(&manager, &id, &DecisionQuery::default())
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(summary_total(&manager, &id).await, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 31);
}

async fn summary_total(manager: &SessionManager, id: &str) -> u64 {
    summary(manager, id).await.unwrap().total
}

async fn raw_pool(db: &std::path::Path) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_schema_20_store_upgrades_and_keeps_its_chats() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("sessions").join("sessions.db");
    let id = {
        let manager = SessionManager::new(dir.path().into());
        let id = chat(&manager, &dir).await;
        manager.close().await;
        id
    };
    // Back to what a build before the decision log left on disk.
    {
        let pool = raw_pool(&db).await;
        sqlx::query("DROP TABLE decisions")
            .execute(&pool)
            .await
            .unwrap();
        // A fresh store records only the version it was created at, so the
        // counter is restamped rather than trimmed.
        sqlx::query("DELETE FROM schema_version")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (20)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let manager = Arc::new(SessionManager::new(dir.path().into()));
    let listed = manager.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1, "the chat survived the upgrade");
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].message_count, 1);
    assert_eq!(listed[0].decision_count, None);
    let conversation = manager
        .get_session(&id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    assert_eq!(
        conversation.messages()[0].as_concat_text(),
        "Find the MS patients."
    );

    // The upgraded store takes rows, and lists them on the chat.
    let service = DecisionService::new(Arc::new(OneModel {
        calls: Arc::new(AtomicUsize::new(0)),
    }))
    .with_default_model(None)
    .with_use_gate(Arc::new(NoCrew))
    .with_sink(Arc::new(SqliteDecisionSink {
        session_manager: Arc::clone(&manager),
    }));
    service
        .decide(
            Some(&DecisionSession::new(
                &id,
                SessionClassification::Public,
                true,
            )),
            None,
            &DecisionRequest {
                questions: has_ms(),
                context: DecisionContext::Text("MS".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(summary_total(&manager, &id).await, 1);
    manager.close().await;

    let pool = raw_pool(&db).await;
    let versions: Vec<i64> = sqlx::query_scalar("SELECT version FROM schema_version")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(versions.contains(&21), "arm 21 ran: {versions:?}");
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'decisions' \
         ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for wanted in [
        "idx_decisions_batch",
        "idx_decisions_origin",
        "idx_decisions_session",
        "idx_decisions_turn",
    ] {
        assert!(indexes.iter().any(|i| i == wanted), "{wanted}: {indexes:?}");
    }
    pool.close().await;
}

/// Arm 21 never runs on a store a development build already stamped 21; the
/// reconcile creates the table there anyway.
#[tokio::test]
async fn a_store_stamped_21_without_the_table_still_gets_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("sessions").join("sessions.db");
    let id = {
        let manager = SessionManager::new(dir.path().into());
        let id = chat(&manager, &dir).await;
        manager.close().await;
        id
    };
    {
        let pool = raw_pool(&db).await;
        sqlx::query("DROP TABLE decisions")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
    let manager = SessionManager::new(dir.path().into());
    assert_eq!(summary_total(&manager, &id).await, 0);
    let listed = manager.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1, "the full listing reads the table");
    manager.close().await;
}
