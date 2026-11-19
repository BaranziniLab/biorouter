//! Focused US-11 / CREW-28 accounting tests. The HTTP caller is authenticated;
//! the standing Crew grants and every model answer are synthetic fixtures.
//! These tests do not establish broker authentication, microphone behavior, or
//! native inference. Historical US-11 EXPLAINED and CREW-28 BLOCKED stay separate.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use biorouter::config::paths::Paths;
use biorouter::session::{SessionType, SideUsageEntry};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "focused-crew-usage-daemon-secret";
const USER_KEY: &str = "focused-crew-usage-person-proof";
const CONNECTION: &str = "focused-crew-usage-connection";

#[ctor::ctor]
fn keep_crew_credentials_in_a_synthetic_profile() {
    let root = std::env::temp_dir().join(format!(
        "biorouter-focused-crew-usage-profile-{}",
        std::process::id()
    ));
    let home = root.join("home");
    let profile = root.join("profile");
    if std::fs::create_dir_all(&home)
        .and_then(|()| std::fs::create_dir_all(&profile))
        .is_err()
    {
        std::process::abort();
    }
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", home);
    std::env::set_var("BIOROUTER_DEV_PROFILE_ROOT", profile);
}

#[ctor::dtor]
fn remove_the_synthetic_profile() {
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!(
        "biorouter-focused-crew-usage-profile-{}",
        std::process::id()
    )));
}

struct Fixture {
    state: Arc<AppState>,
    crew_chat: String,
    unrelated_chat: String,
}

async fn fixture() -> Fixture {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-User-Action", USER_KEY.parse().unwrap());
    assert!(biorouter_server::auth::is_user_action(&headers));
    assert_eq!(
        std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref(),
        Ok("true")
    );
    assert!(
        std::path::Path::new(&std::env::var("BIOROUTER_DEV_PROFILE_ROOT").unwrap()).is_absolute()
    );
    let state = AppState::new().await.unwrap();
    let manager = state.session_manager();
    let crew_chat = manager
        .create_session(
            Paths::data_dir(),
            "Focused Crew usage fixture".into(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;
    let unrelated_chat = manager
        .create_session(
            Paths::data_dir(),
            "Unrelated accounting control".into(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;
    let crew_root = Paths::config_dir().join("crew");
    std::fs::create_dir_all(&crew_root).unwrap();
    let registry = crew_root.join("connections.json");
    assert!(
        !registry.exists(),
        "the fixture must precede Crew registry loading"
    );
    std::fs::write(
        registry,
        serde_json::to_vec(&json!({
            "connections": [{
                "id": CONNECTION,
                "name": "Synthetic usage workspace",
                "ssh_target": "fixture@crew.invalid",
                "port": null,
                "identity_file": null,
                "proxy_jump": null,
                "socket_path": "/tmp/focused-crew-usage.sock",
                "owner_uid": 10001,
                "workspace_id": "focused-crew-usage-workspace",
                "workspace_public_key": "00".repeat(32),
                "cluster_connection_id": "focused-crew-usage-cluster",
                "mode": "public",
                "policy_epoch": 1,
                "status": "disconnected",
                "last_error": null,
                "device_id": "11".repeat(32),
                "public_key": "22".repeat(32)
            }],
            "scopes": {
                crew_chat.as_str(): {
                    "connection_id": CONNECTION,
                    "run_id": "focused-crew-usage-run",
                    "channel_id": "focused-crew-usage-channel",
                    "source_channels": ["focused-crew-usage-channel"],
                    "epoch": 1,
                    "provider_binding": "focused-crew-usage-provider",
                    "public_provider": true,
                    "institution_policy": true,
                    "expired": false,
                    "expires_at": 4_000_000_000_u64
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let crew = biorouter::crew::manager().unwrap();
    assert!(crew.is_scoped_session(&crew_chat).await);
    assert!(crew.history_rewrite_refusal(&crew_chat).await.is_none());
    assert!(!crew.is_scoped_session(&unrelated_chat).await);
    Fixture {
        state,
        crew_chat,
        unrelated_chat,
    }
}

async fn http(
    fixture: &Fixture,
    method: &str,
    uri: &str,
    body: Option<Value>,
    proven: bool,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET);
    if proven {
        request = request.header("X-User-Action", USER_KEY);
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = biorouter_server::routes::configure(fixture.state.clone(), SECRET.into())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn crew_usage_tiny_monthly_limit_refuses_start_without_a_run_receipt() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let fixture = fixture().await;
    fixture
        .state
        .session_manager()
        .apply_side_usage_event(SideUsageEntry {
            event_key: "focused-crew-start-limit-seed".into(),
            session_id: fixture.crew_chat.clone(),
            provider: "focused-crew-usage-provider".into(),
            model_id: "synthetic".into(),
            input_tokens: Some(1),
            output_tokens: Some(0),
        })
        .await
        .unwrap();
    let (status, body) = http(
        &fixture,
        "PUT",
        "/usage/limits",
        Some(json!({"monthlyTokenLimit": 1})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uri = format!("/crew/connections/{CONNECTION}/runs");
    let request = json!({
        "request_id": "focused-start-refusal",
        "channel_id": "focused-crew-usage-channel",
        "prompt": "Return the synthetic fixture result.",
        "provider": "must-not-be-opened-at-the-limit",
        "model": "synthetic",
        "posting_grant": true
    });
    let (status, body) = http(&fixture, "POST", &uri, Some(request.clone()), false).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = http(&fixture, "POST", &uri, Some(request.clone()), true).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["code"], "crew_usage_limit_reached");
    assert_eq!(
        body["error"],
        biorouter::agents::usage_limit::CREW_TASK_MONTH_LIMIT
    );
    let (status, body) = http(&fixture, "GET", &uri, None, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["runs"].as_array().unwrap().is_empty(), "{body}");
    let ledger = Paths::state_dir().join("crew/runs.json");
    if ledger.exists() {
        let saved: Value = serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
        assert!(saved["requests"].as_object().unwrap().is_empty());
        assert!(saved["runs"].as_array().unwrap().is_empty());
    }
    let (status, body) = http(
        &fixture,
        "PUT",
        "/usage/limits",
        Some(json!({"monthlyTokenLimit": null})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = http(&fixture, "POST", &uri, Some(request), true).await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(
        body.to_string().contains("must-not-be-opened-at-the-limit"),
        "lifting the limit must reach provider resolution: {body}"
    );
}

mod focused_decision_ledger {
    use super::{fixture, http};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use biorouter::decision::log::{self, DecisionQuery, SqliteDecisionSink};
    use biorouter::decision::types::{
        DecisionKindTag, ModelVerdicts, ProbabilitySource, QuestionVerdict,
    };
    use biorouter::decision::{
        monitor, settle_late_answers, turns, DecisionAdapter, DecisionCaps, DecisionContext,
        DecisionError, DecisionKind, DecisionLogEntry, DecisionModel, DecisionModelEntry,
        DecisionModelRef, DecisionModelSource, DecisionOrigin, DecisionOutcome, DecisionRequest,
        DecisionResult, DecisionService, DecisionSession, DecisionSink, DecisionUsage, Question,
        Threshold,
    };
    use biorouter::privacy::affiliation::ModelAffiliation;
    use biorouter::privacy::{ProviderTier, SessionClassification};
    use biorouter::session::SessionManager;
    use serde_json::json;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tokio::sync::Notify;

    const PROVIDER: &str = "ollama_decision";
    const MODEL: &str = "tev1:0.8b";
    const INPUT: i64 = 42;
    const OUTPUT: i64 = 3;
    const WAIT: Duration = Duration::from_secs(5);

    fn model_ref() -> DecisionModelRef {
        DecisionModelRef::new(PROVIDER, MODEL).with_adapter(DecisionAdapter::SystemOne)
    }

    #[derive(Default)]
    struct AnswerControl {
        entered: Notify,
        release: Notify,
        calls: AtomicUsize,
    }

    struct LocalChecker(Arc<AnswerControl>);

    #[async_trait]
    impl DecisionModel for LocalChecker {
        fn model_ref(&self) -> DecisionModelRef {
            model_ref()
        }
        fn display_name(&self) -> String {
            "synthetic local checker".into()
        }
        fn tier(&self) -> ProviderTier {
            ProviderTier::Private
        }
        fn affiliation(&self) -> Option<ModelAffiliation> {
            Some(ModelAffiliation::Local)
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: vec![DecisionKindTag::Binary],
                max_questions: 1,
                context_tokens: 8192,
                max_concurrency: 1,
                max_request_bytes: None,
                local: true,
            }
        }
        async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0.entered.notify_one();
            self.0.release.notified().await;
            Ok(ModelVerdicts {
                verdicts: request
                    .questions
                    .keys()
                    .map(|name| QuestionVerdict {
                        question: name.clone(),
                        probabilities: [("yes".into(), 0.99), ("no".into(), 0.01)]
                            .into_iter()
                            .collect(),
                        source: ProbabilitySource::Native,
                        calibrated: true,
                        reported_choice: None,
                        raw: json!({"synthetic": true}),
                    })
                    .collect(),
                usage: DecisionUsage {
                    input_tokens: Some(INPUT),
                    output_tokens: Some(OUTPUT),
                },
                served_model: Some("synthetic-local-copy".into()),
            })
        }
    }

    struct LocalSource(Arc<LocalChecker>);

    #[async_trait]
    impl DecisionModelSource for LocalSource {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            vec![DecisionModelEntry {
                provider: PROVIDER.into(),
                model: MODEL.into(),
                display_name: "synthetic local checker".into(),
                context_tokens: 8192,
                kinds: vec![DecisionKindTag::Binary],
                tier: ProviderTier::Private,
                affiliation: Some(ModelAffiliation::Local),
                adapter: DecisionAdapter::SystemOne,
                local: true,
                load_refusal: None,
            }]
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            if wanted.provider != PROVIDER || wanted.model != MODEL {
                return Err(DecisionError::Invalid(
                    "unknown synthetic decision model".into(),
                ));
            }
            Ok(self.0.clone())
        }
    }

    struct ObservedSink {
        inner: SqliteDecisionSink,
        normal_writes: AtomicUsize,
        late_writes: AtomicUsize,
        late_done: Notify,
        late_entry: Mutex<Option<(String, DecisionLogEntry)>>,
        late_error: Mutex<Option<String>>,
    }

    #[async_trait]
    impl DecisionSink for ObservedSink {
        async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.inner.record(entry).await?;
            self.normal_writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn record_as(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.inner.record_as(id, entry).await?;
            self.normal_writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn record_late(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
            let result = self.inner.record_late(id, entry.clone()).await;
            *self.late_entry.lock().unwrap() = Some((id.into(), entry));
            *self.late_error.lock().unwrap() = result.as_ref().err().map(ToString::to_string);
            self.late_writes.fetch_add(1, Ordering::SeqCst);
            // Completion follows the real SQLite write, not merely the model's answer.
            self.late_done.notify_one();
            result
        }
    }

    fn service(
        deadline: Duration,
    ) -> (Arc<DecisionService>, Arc<AnswerControl>, Arc<ObservedSink>) {
        let control = Arc::new(AnswerControl::default());
        let sink = Arc::new(ObservedSink {
            inner: SqliteDecisionSink {
                session_manager: Arc::new(SessionManager::instance()),
            },
            normal_writes: AtomicUsize::new(0),
            late_writes: AtomicUsize::new(0),
            late_done: Notify::new(),
            late_entry: Mutex::new(None),
            late_error: Mutex::new(None),
        });
        let svc = DecisionService::new(Arc::new(LocalSource(Arc::new(LocalChecker(
            control.clone(),
        )))))
        .with_default_model(Some(model_ref()))
        .with_timeouts(deadline, deadline)
        .with_sink(sink.clone());
        // Keep the default CrewUseGate: do not install a permissive test gate.
        (Arc::new(svc), control, sink)
    }

    fn request() -> DecisionRequest {
        DecisionRequest {
            questions: [(
                "marker".into(),
                Question {
                    instructions: "Is the synthetic marker present?".into(),
                    kind: DecisionKind::Binary {},
                    threshold: Some(Threshold::Yes(0.9)),
                },
            )]
            .into_iter()
            .collect(),
            context: DecisionContext::Text("SYNTHETIC MARKER: present".into()),
        }
    }

    fn context(chat: &str, turn: &str, origin: &str) -> DecisionSession {
        DecisionSession::new(chat, SessionClassification::Public, true)
            .with_origin(DecisionOrigin::Tool, Some(origin.into()))
            .with_origin_label(Some("focused Crew decision".into()))
            .with_turn(Some(turn.into()))
    }

    async fn raw_pool() -> sqlx::SqlitePool {
        let db = SessionManager::shared_store_root()
            .join("sessions")
            .join("sessions.db");
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(db)
                    .create_if_missing(false)
                    .read_only(true)
                    .busy_timeout(WAIT),
            )
            .await
            .unwrap()
    }

    type Event = (String, String, String, Option<i64>, Option<i64>);
    async fn events(pool: &sqlx::SqlitePool, id: &str) -> Vec<Event> {
        sqlx::query_as(
            "SELECT session_id, provider, model_id, input_tokens, output_tokens \
                         FROM token_events WHERE event_key = ?",
        )
        .bind(format!("decision:{id}"))
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn totals(manager: &SessionManager, chat: &str) -> (i64, i64, i64) {
        let row = manager.get_session(chat, false).await.unwrap();
        (
            row.accumulated_input_tokens.unwrap_or(0),
            row.accumulated_output_tokens.unwrap_or(0),
            row.accumulated_total_tokens.unwrap_or(0),
        )
    }

    async fn assert_single_entry(
        manager: &SessionManager,
        pool: &sqlx::SqlitePool,
        chat: &str,
        unrelated: &str,
        id: &str,
        (turn, origin): (&str, &str),
        outcome: DecisionOutcome,
    ) {
        let query = DecisionQuery {
            decision_id: Some(id.into()),
            ..Default::default()
        };
        let page = log::list(manager, chat, &query).await.unwrap();
        assert_eq!((page.total, page.records.len()), (1, 1));
        let row = &page.records[0];
        assert_eq!(row.id, id);
        assert_eq!(row.session_id.as_deref(), Some(chat));
        assert_eq!(row.turn_id.as_deref(), Some(turn));
        assert_eq!(row.origin, DecisionOrigin::Tool);
        assert_eq!(row.origin_id.as_deref(), Some(origin));
        assert_eq!(row.label.as_deref(), Some("focused Crew decision"));
        assert_eq!(row.outcome, outcome);
        assert_eq!(row.model.as_ref(), Some(&model_ref()));
        assert_eq!(
            (row.input_tokens, row.output_tokens),
            (Some(INPUT), Some(OUTPUT))
        );
        assert!(row.privacy_enforced);
        assert_eq!(
            log::get(manager, chat, id).await.unwrap().unwrap().context,
            request().context
        );
        assert_eq!(
            log::list(manager, unrelated, &query).await.unwrap().total,
            0
        );
        let physical: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(physical, 1);
        assert_eq!(
            events(pool, id).await,
            vec![(
                chat.into(),
                PROVIDER.into(),
                MODEL.into(),
                Some(INPUT),
                Some(OUTPUT)
            )]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn a_crew_decision_records_one_row_and_one_usage_event() {
        if !super::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let f = fixture().await;
        let manager = f.state.session_manager();
        let before = totals(manager, &f.crew_chat).await;
        let unrelated_before = totals(manager, &f.unrelated_chat).await;
        let crew = biorouter::crew::manager().unwrap();
        assert!(crew.is_scoped_session(&f.crew_chat).await);
        assert_eq!(crew.history_rewrite_refusal(&f.crew_chat).await, None);
        let id = format!("crew-ledger-success-{}", uuid::Uuid::new_v4());
        let turn = format!("{id}-turn");
        let origin = format!("{id}-tool");
        let (svc, control, sink) = service(WAIT);
        let session = context(&f.crew_chat, &turn, &origin);
        assert_eq!(svc.session_refusal(&session).await, None);
        turns::begin_turn(&f.crew_chat, &turn);
        let call = tokio::spawn({
            let svc = svc.clone();
            let id = id.clone();
            async move {
                svc.decide_as(Some(&session), Some(&model_ref()), &request(), &id)
                    .await
            }
        });
        tokio::time::timeout(WAIT, control.entered.notified())
            .await
            .unwrap();
        control.release.notify_one();
        let decisions = tokio::time::timeout(WAIT, call)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(decisions.len(), 1);
        let pool = raw_pool().await;
        assert_single_entry(
            manager,
            &pool,
            &f.crew_chat,
            &f.unrelated_chat,
            &id,
            (&turn, &origin),
            DecisionOutcome::Passed,
        )
        .await;
        assert_eq!(
            totals(manager, &f.crew_chat).await,
            (
                before.0 + INPUT,
                before.1 + OUTPUT,
                before.2 + INPUT + OUTPUT
            )
        );
        assert_eq!(totals(manager, &f.unrelated_chat).await, unrelated_before);
        let (status, body) = http(
            &f,
            "GET",
            &format!("/sessions/{}/decisions/{id}", f.crew_chat),
            None,
            true,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["record"]["id"], id);
        assert_eq!(control.calls.load(Ordering::SeqCst), 1);
        assert_eq!(sink.normal_writes.load(Ordering::SeqCst), 1);
        assert_eq!(sink.late_writes.load(Ordering::SeqCst), 0);
        turns::end_turn(&f.crew_chat, &turn);
        let tally = monitor::take_turn_tally(&turn);
        assert_eq!(tally.len(), 1);
        assert_eq!((tally[0].count, tally[0].calls, tally[0].passed), (1, 1, 1));
        pool.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn a_late_crew_decision_bills_its_original_session_and_turn_once() {
        if !super::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let f = fixture().await;
        let manager = f.state.session_manager();
        let before = totals(manager, &f.crew_chat).await;
        let unrelated_before = totals(manager, &f.unrelated_chat).await;
        let crew = biorouter::crew::manager().unwrap();
        assert!(crew.is_scoped_session(&f.crew_chat).await);
        assert_eq!(crew.history_rewrite_refusal(&f.crew_chat).await, None);
        let id = format!("crew-ledger-late-{}", uuid::Uuid::new_v4());
        let old_turn = format!("{id}-old");
        let successor = format!("{id}-successor");
        let other_turn = format!("{id}-unrelated");
        let origin = format!("{id}-tool");
        let (svc, control, sink) = service(Duration::from_millis(250));
        let session = context(&f.crew_chat, &old_turn, &origin);
        turns::begin_turn(&f.crew_chat, &old_turn);
        let call = tokio::spawn({
            let svc = svc.clone();
            let id = id.clone();
            async move {
                svc.decide_as(Some(&session), Some(&model_ref()), &request(), &id)
                    .await
            }
        });
        tokio::time::timeout(WAIT, control.entered.notified())
            .await
            .unwrap();
        let failure = tokio::time::timeout(WAIT, call)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(failure, DecisionError::Timeout { .. }),
            "{failure:?}"
        );
        let pool = raw_pool().await;
        let timeout_row = log::get(manager, &f.crew_chat, &id)
            .await
            .unwrap()
            .unwrap()
            .record;
        assert_eq!(timeout_row.outcome, DecisionOutcome::Error);
        assert_eq!(timeout_row.turn_id.as_deref(), Some(old_turn.as_str()));
        assert_eq!(
            (timeout_row.input_tokens, timeout_row.output_tokens),
            (None, None)
        );
        assert!(timeout_row.decisions.is_empty());
        assert!(events(&pool, &id).await.is_empty());
        turns::end_turn(&f.crew_chat, &old_turn);
        turns::begin_turn(&f.crew_chat, &successor);
        turns::begin_turn(&f.unrelated_chat, &other_turn);
        control.release.notify_one();
        tokio::time::timeout(WAIT, sink.late_done.notified())
            .await
            .unwrap();
        assert_eq!(*sink.late_error.lock().unwrap(), None);
        assert_single_entry(
            manager,
            &pool,
            &f.crew_chat,
            &f.unrelated_chat,
            &id,
            (&old_turn, &origin),
            DecisionOutcome::Error,
        )
        .await;
        assert_eq!(
            totals(manager, &f.crew_chat).await,
            (
                before.0 + INPUT,
                before.1 + OUTPUT,
                before.2 + INPUT + OUTPUT
            )
        );
        assert_eq!(totals(manager, &f.unrelated_chat).await, unrelated_before);
        let row = log::get(manager, &f.crew_chat, &id)
            .await
            .unwrap()
            .unwrap()
            .record;
        assert!(row.decisions.is_empty(), "a late verdict was not used");
        assert!(row.error.as_deref().unwrap().contains("too late to use"));
        assert_eq!(
            turns::current_turn(&f.crew_chat).as_deref(),
            Some(successor.as_str())
        );
        assert_eq!(
            turns::current_turn(&f.unrelated_chat).as_deref(),
            Some(other_turn.as_str())
        );
        assert!(monitor::take_turn_tally(&successor).is_empty());
        assert!(monitor::take_turn_tally(&other_turn).is_empty());
        let tally = monitor::take_turn_tally(&old_turn);
        assert_eq!(tally.len(), 1);
        assert_eq!((tally[0].count, tally[0].calls, tally[0].errors), (1, 1, 1));
        let (status, body) = http(
            &f,
            "GET",
            &format!("/sessions/{}/decisions/{id}", f.crew_chat),
            None,
            true,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["record"]["id"], id);
        assert_eq!(control.calls.load(Ordering::SeqCst), 1);
        assert_eq!(sink.normal_writes.load(Ordering::SeqCst), 1);
        assert_eq!(sink.late_writes.load(Ordering::SeqCst), 1);
        let replay = sink.late_entry.lock().unwrap().clone().unwrap();
        assert_eq!(replay.0, id);
        assert_eq!(replay.1.session_id.as_deref(), Some(f.crew_chat.as_str()));
        assert_eq!(replay.1.turn_id.as_deref(), Some(old_turn.as_str()));
        // A repeated delivery exercises the real sink's row/usage idempotency.
        sink.inner.record_late(&replay.0, replay.1).await.unwrap();
        assert_single_entry(
            manager,
            &pool,
            &f.crew_chat,
            &f.unrelated_chat,
            &id,
            (&old_turn, &origin),
            DecisionOutcome::Error,
        )
        .await;
        assert_eq!(
            totals(manager, &f.crew_chat).await,
            (
                before.0 + INPUT,
                before.1 + OUTPUT,
                before.2 + INPUT + OUTPUT
            )
        );
        assert_eq!(totals(manager, &f.unrelated_chat).await, unrelated_before);
        let successor_rows = log::list(
            manager,
            &f.crew_chat,
            &DecisionQuery {
                turn_id: Some(successor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(successor_rows.total, 0);
        turns::end_turn(&f.crew_chat, &successor);
        turns::end_turn(&f.unrelated_chat, &other_turn);
        // Local inference is never run; this fake's model has no priced-dollar claim.
        assert_eq!(settle_late_answers(WAIT).await, 0);
        pool.close().await;
    }
}
