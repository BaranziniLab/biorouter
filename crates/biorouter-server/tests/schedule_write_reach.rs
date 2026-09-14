//! Issue #56: every route that creates, runs, re-times, pauses, resumes or
//! removes a schedule asks `session_reach::schedule_reach` first, over the real
//! router tree, with the headers each caller really sends.
//!
//! Measured on `main` (1038a113) against a sandboxed daemon configured with a
//! private model, holding nothing but `X-Secret-Key`: `POST /schedule/create`
//! 200, `PUT /schedule/{id}` 200, `pause`/`unpause` 204, `DELETE` 204, and
//! `POST /schedule/{id}/run_now` 200 with a new chat on `versa_azure` that `GET
//! /sessions/{that id}` then refused the same caller. `PUT` with the job's own
//! cron handed back `creator_session_id` naming a private chat that `GET
//! /schedule/list` redacts. Every refusal below failed against that tree.
//!
//! ⚠ **Its own binary, and ONE test in it, on purpose.** Seeding a schedule
//! registers a task on tokio-cron-scheduler, which is process-global while each
//! `#[tokio::test]` brings its own runtime; a second test's add fails with
//! `CantAdd` (measured, and recorded beside `redact_unreachable_chats`). The
//! user-action digest is a process-global `OnceLock` too, and phase B moves
//! `BIOROUTER_PROVIDER`, which no other test may see.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{body::Body, http::Request, Router};
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::scheduler::ScheduledJob;
use biorouter::session::SessionType;
use biorouter_server::routes::session_reach::{CALLER_PROVIDER_HEADER, SCHEDULE_OUT_OF_REACH};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const USER_ACTION_KEY: &str = "schedule-write-reach-user-action-key";
/// The proof-of-user header, as the desktop app sends it.
const PROOF: (&str, &str) = ("X-User-Action", USER_ACTION_KEY);
/// A caller stating it runs under an institution-hosted model — the CLI's
/// shape on an install configured with one.
const PRIVATE_CAPABILITY: (&str, &str) = (CALLER_PROVIDER_HEADER, "versa_azure");
/// Midnight on the first of January: no schedule here fires while it runs.
const FAR_CRON: &str = "0 0 0 1 1 *";

struct Probe {
    app: Router,
    state: Arc<AppState>,
    mismatches: Vec<String>,
}

impl Probe {
    async fn send(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> (u16, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let body = match body {
            Some(json) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&json).unwrap())
            }
            None => Body::empty(),
        };
        let res = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The request is refused in the schedule's words, and nothing else.
    async fn refused(&mut self, label: &str, method: &str, uri: &str, body: Option<Value>) {
        let answer = self.send(method, uri, body, &[]).await;
        if answer != (403, SCHEDULE_OUT_OF_REACH.to_string()) {
            self.mismatches.push(format!(
                "{label}: a secret-only caller got {} {}",
                answer.0,
                answer.1.chars().take(240).collect::<String>()
            ));
        }
    }

    /// The request got past the gate — whatever the route then said.
    async fn admitted(
        &mut self,
        label: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
        expected: u16,
    ) -> String {
        let (status, answer) = self.send(method, uri, body, headers).await;
        if status != expected || answer == SCHEDULE_OUT_OF_REACH {
            self.mismatches.push(format!(
                "{label} {headers:?}: wanted {expected}, got {status} {}",
                answer.chars().take(240).collect::<String>()
            ));
        }
        answer
    }

    async fn job(&self, id: &str) -> Option<ScheduledJob> {
        self.state
            .scheduler()
            .list_scheduled_jobs()
            .await
            .into_iter()
            .find(|job| job.id == id)
    }

    fn check(&mut self, ok: bool, what: String) {
        if !ok {
            self.mismatches.push(what);
        }
    }
}

async fn seed_chat(state: &AppState, label: &str, provider: &str, private: bool) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            PathBuf::from("/tmp/schedule_write_reach"),
            label.to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    let mut update = manager
        .update(&session.id)
        .provider_name(provider)
        .model_config(ModelConfig::new("schedule-write-reach-model").unwrap());
    if private {
        update = update.raise_privacy(SessionClassification::Private, "turn:versa_azure");
    }
    update.apply().await.unwrap();
    session.id
}

fn job(id: &str, source: &Path, creator: Option<&str>) -> ScheduledJob {
    ScheduledJob {
        id: id.to_string(),
        source: source.to_string_lossy().into_owned(),
        cron: FAR_CRON.to_string(),
        last_run: None,
        currently_running: false,
        paused: false,
        current_session_id: None,
        process_start_time: None,
        run_count: 0,
        max_runs: None,
        creator_session_id: creator.map(str::to_owned),
        last_error: None,
        owns_source: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_schedules_private_work_is_changed_only_by_a_caller_that_could_reach_it() {
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    // PHASE A is an install with no configured model at all, so a schedule that
    // names no chat runs public work.
    std::env::remove_var("BIOROUTER_PROVIDER");
    std::env::remove_var("BIOROUTER_MODEL");
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));

    let state = AppState::new().await.unwrap();
    let app = biorouter_server::routes::configure(state.clone(), "schedule-write-reach".into());
    let mut probe = Probe {
        app,
        state: state.clone(),
        mismatches: Vec::new(),
    };

    let private_chat = seed_chat(
        &state,
        "Schedule reach private (fixture)",
        "versa_azure",
        true,
    )
    .await;
    let public_chat = seed_chat(
        &state,
        "Schedule reach public (fixture)",
        "anthropic",
        false,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let workflow = dir.path().join("probe.yaml");
    std::fs::write(
        &workflow,
        "version: 1.0.0\ntitle: Schedule reach probe\ndescription: probe\n\
         prompt: Reply with the single word OK.\n",
    )
    .unwrap();
    let scheduler = state.scheduler();
    for (id, creator) in [
        ("sr-nameless", None),
        ("sr-public-creator", Some(public_chat.as_str())),
        ("sr-private-creator", Some(private_chat.as_str())),
    ] {
        scheduler
            .add_scheduled_job(job(id, &workflow, creator), true)
            .await
            .unwrap_or_else(|e| panic!("seeding {id}: {e}"));
    }

    // ── PHASE A: a public default. ──────────────────────────────────────────

    // A schedule whose work is public is untouched by the gate — the whole
    // Schedules surface of a public install, and the CLI on it.
    for id in ["sr-nameless", "sr-public-creator"] {
        let put = probe
            .admitted(
                &format!("PUT {id}"),
                "PUT",
                &format!("/schedule/{id}"),
                Some(json!({ "cron": FAR_CRON })),
                &[],
                200,
            )
            .await;
        if id == "sr-public-creator" {
            probe.check(
                put.contains(&public_chat),
                format!("PUT {id} redacted a PUBLIC creator from a secret-only caller: {put}"),
            );
        }
        for action in ["pause", "unpause"] {
            probe
                .admitted(
                    &format!("{action} {id}"),
                    "POST",
                    &format!("/schedule/{id}/{action}"),
                    None,
                    &[],
                    204,
                )
                .await;
        }
    }
    // Admitted, and then refused by the scheduler itself: with no model
    // configured anywhere the run cannot bind one. What matters here is that
    // the answer is the scheduler's and not the gate's.
    probe
        .admitted(
            "run_now sr-nameless",
            "POST",
            "/schedule/sr-nameless/run_now",
            None,
            &[],
            500,
        )
        .await;
    probe
        .admitted(
            "create on a public default",
            "POST",
            "/schedule/create",
            Some(json!({
                "id": "sr-created-public",
                "workflow_source": workflow,
                "cron": FAR_CRON,
            })),
            &[],
            200,
        )
        .await;

    // A schedule made FROM a private chat does that chat's work, on that chat's
    // model — and an id that names no schedule is answered the same way.
    for id in ["sr-private-creator", "sr-no-such-schedule"] {
        probe
            .refused(
                &format!("PUT {id}"),
                "PUT",
                &format!("/schedule/{id}"),
                Some(json!({ "cron": FAR_CRON })),
            )
            .await;
        for action in ["pause", "unpause", "run_now"] {
            probe
                .refused(
                    &format!("{action} {id}"),
                    "POST",
                    &format!("/schedule/{id}/{action}"),
                    None,
                )
                .await;
        }
        probe
            .refused(
                &format!("DELETE {id}"),
                "DELETE",
                &format!("/schedule/delete/{id}"),
                None,
            )
            .await;
    }
    let untouched = probe.job("sr-private-creator").await;
    probe.check(
        untouched
            .as_ref()
            .is_some_and(|job| !job.paused && job.last_run.is_none() && job.last_error.is_none()),
        format!("a refused caller changed the private chat's schedule: {untouched:?}"),
    );

    // The person at the keyboard, and a program on a private model, reach it —
    // and the person is told the truth about an id that names nothing.
    for headers in [&[PROOF][..], &[PRIVATE_CAPABILITY][..]] {
        for action in ["pause", "unpause"] {
            probe
                .admitted(
                    &format!("{action} sr-private-creator"),
                    "POST",
                    &format!("/schedule/sr-private-creator/{action}"),
                    None,
                    headers,
                    204,
                )
                .await;
        }
    }
    let put = probe
        .admitted(
            "PUT sr-private-creator",
            "PUT",
            "/schedule/sr-private-creator",
            Some(json!({ "cron": FAR_CRON })),
            &[PROOF],
            200,
        )
        .await;
    probe.check(
        put.contains(&private_chat),
        format!("the person lost the private creator from the job PUT answers with: {put}"),
    );
    probe
        .admitted(
            "pause sr-no-such-schedule",
            "POST",
            "/schedule/sr-no-such-schedule/pause",
            None,
            &[PROOF],
            404,
        )
        .await;

    // ── PHASE B: the install's default model is private. ────────────────────
    std::env::set_var("BIOROUTER_PROVIDER", "versa_azure");
    std::env::set_var("BIOROUTER_MODEL", "gpt-4o");

    // A schedule that names no chat now runs on a private model with no person
    // present: the bind `POST /agent/start` refuses this caller (SD-12).
    probe
        .refused(
            "create on a private default",
            "POST",
            "/schedule/create",
            Some(json!({
                "id": "sr-created-private",
                "workflow_source": workflow,
                "cron": FAR_CRON,
            })),
        )
        .await;
    let created = probe.job("sr-created-private").await;
    probe.check(
        created.is_none(),
        format!("a refused create still made the schedule: {created:?}"),
    );
    for action in ["pause", "unpause", "run_now"] {
        probe
            .refused(
                &format!("{action} sr-nameless on a private default"),
                "POST",
                &format!("/schedule/sr-nameless/{action}"),
                None,
            )
            .await;
    }
    for (id, headers) in [
        ("sr-created-by-person", &[PROOF][..]),
        ("sr-created-by-program", &[PRIVATE_CAPABILITY][..]),
    ] {
        probe
            .admitted(
                &format!("create {id}"),
                "POST",
                "/schedule/create",
                Some(json!({ "id": id, "workflow_source": workflow, "cron": FAR_CRON })),
                headers,
                200,
            )
            .await;
    }
    // A schedule made from a PUBLIC chat runs on that chat's public model, not
    // on the default, so it stays open: the gate reads the model the run binds.
    for action in ["pause", "unpause"] {
        probe
            .admitted(
                &format!("{action} sr-public-creator on a private default"),
                "POST",
                &format!("/schedule/sr-public-creator/{action}"),
                None,
                &[],
                204,
            )
            .await;
    }

    // `POST /workflows/schedule` reaches the same scheduler by a workflow's id.
    // A workflow in a library root of its own, found the way the Workflows page
    // finds one: by listing, which hands back the id the route resolves.
    let library = tempfile::tempdir().unwrap();
    std::fs::write(
        library.path().join("schedule-reach-library-probe.yaml"),
        "version: 1.0.0\ntitle: Schedule reach library probe\ndescription: probe\n\
         prompt: Reply with the single word OK.\n",
    )
    .unwrap();
    std::env::set_var("BIOROUTER_WORKFLOW_PATH", library.path());
    let (status, listed) = probe.send("GET", "/workflows/list", None, &[PROOF]).await;
    assert_eq!(status, 200, "listing workflows: {listed}");
    let workflow_id = serde_json::from_str::<Value>(&listed).unwrap()["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|manifest| {
            manifest["file_path"]
                .as_str()
                .is_some_and(|path| path.ends_with("schedule-reach-library-probe.yaml"))
        })
        .unwrap_or_else(|| panic!("the probe workflow is not listed: {listed}"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let schedule_ids = |state: Arc<AppState>| async move {
        state
            .scheduler()
            .list_scheduled_jobs()
            .await
            .into_iter()
            .map(|job| job.id)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let before = schedule_ids(state.clone()).await;
    probe
        .refused(
            "POST /workflows/schedule (add)",
            "POST",
            "/workflows/schedule",
            Some(json!({ "id": workflow_id, "cron_schedule": FAR_CRON })),
        )
        .await;
    let after_refusal = schedule_ids(state.clone()).await;
    probe.check(
        after_refusal == before,
        format!("a refused /workflows/schedule still scheduled the workflow: {after_refusal:?}"),
    );
    probe
        .admitted(
            "POST /workflows/schedule (add)",
            "POST",
            "/workflows/schedule",
            Some(json!({ "id": workflow_id, "cron_schedule": FAR_CRON })),
            &[PROOF],
            200,
        )
        .await;
    let with_library = schedule_ids(state.clone()).await;
    probe.check(
        with_library.len() == before.len() + 1,
        format!("the person's /workflows/schedule did not schedule the workflow: {with_library:?}"),
    );
    probe
        .refused(
            "POST /workflows/schedule (remove)",
            "POST",
            "/workflows/schedule",
            Some(json!({ "id": workflow_id, "cron_schedule": null })),
        )
        .await;
    let after_removal_refused = schedule_ids(state.clone()).await;
    probe.check(
        after_removal_refused == with_library,
        format!(
            "a refused /workflows/schedule still removed the schedule: {after_removal_refused:?}"
        ),
    );

    // And a refused caller never removed the private chat's schedule.
    let kept = probe.job("sr-private-creator").await;
    probe.check(
        kept.is_some(),
        "the private chat's schedule is gone".to_string(),
    );
    probe
        .admitted(
            "DELETE sr-private-creator",
            "DELETE",
            "/schedule/delete/sr-private-creator",
            None,
            &[PROOF],
            204,
        )
        .await;

    std::env::remove_var("BIOROUTER_PROVIDER");
    std::env::remove_var("BIOROUTER_MODEL");
    std::env::remove_var("BIOROUTER_WORKFLOW_PATH");
    assert!(
        probe.mismatches.is_empty(),
        "{} door(s) answered wrongly:\n{}",
        probe.mismatches.len(),
        probe.mismatches.join("\n")
    );
}
