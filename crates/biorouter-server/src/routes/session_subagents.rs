//! `GET /sessions/{session_id}/subagents`: the subagents one chat has started,
//! with their status, model and setup, and the chat's orchestration plan.
//!
//! The rows come from ONE function, `biorouter::agents::subagent_roster::
//! session_subagents`, which the CLI and the terminal UI also read, so no surface
//! can disagree with another about a child's status. That function applies no
//! privacy filter, because its in-process callers share the child's tier by
//! construction. This route is where the filter is applied for an HTTP caller,
//! in three steps, in this order:
//!
//! 1. **The parent is gated with `session_reach`**, before anything is read, and
//!    a refusal is that gate's exact text. So an unproven caller cannot tell a
//!    private chat from one that does not exist by asking for its subagents.
//! 2. **Each child is filtered with `HttpCaller::lists_session`** on the child's
//!    OWN classification, plus the Crew exclusion every other listing applies. A
//!    child can be more private than its parent (a mid-run raise), and such a row
//!    is omitted, never redacted: its name and result line are content.
//! 3. **The plan is recomputed from the rows that survived**, with
//!    `orchestration_plan::snapshot`, and each phase's `agentIds` is confined to
//!    those rows. A phase's counts are derived from the rows they are given, so a
//!    hidden child is not counted, and its id is not listed, which would otherwise
//!    tell the caller that a private run exists.

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use biorouter::agents::orchestration_plan::OrchestrationSnapshot;
use biorouter::agents::subagent_roster::{SessionSubagents, SubagentRow};

use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/subagents",
    // EXPLICIT tag: this is workspace control's read surface (a chat's
    // delegated work), so the CLI parity gate in
    // `biorouter-cli/src/commands/workspace_parity.rs` must see it, and its
    // counterpart is `biorouter session subagents`.
    tag = "workspace",
    params(
        ("session_id" = String, Path, description = "The parent chat whose subagents to list")
    ),
    responses(
        (status = 200, description = "The chat's subagents, oldest first, and its orchestration \
                                      plan when one was declared. A subagent this caller could \
                                      not open is omitted, never redacted, and is neither listed \
                                      in nor counted by any phase of the plan", body = SessionSubagents),
        (status = 400, description = "Invalid session id"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn get_session_subagents(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !crate::routes::session::is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let manager = state.session_manager();
    // FIRST, before the row is read: the parent's own reach gate. A handler
    // that looked the chat up before this would answer an unproven caller 404
    // for an absent id and something else for a private one.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(manager, &session_id, &headers).await
    {
        return refusal.into_response();
    }
    match manager.get_session(&session_id, false).await {
        Ok(_) => {}
        Err(error) if error.to_string().contains("not found") => {
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }

    // ONE resolved caller for every row, so the rows cannot half-believe two
    // answers.
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    let excluded_crew = match caller.excluded_crew_sessions().await {
        Ok(ids) => ids,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let roster =
        match biorouter::agents::subagent_roster::session_subagents(manager, &session_id).await {
            Ok(roster) => roster,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    let visible = visible_roster(roster, |row| {
        caller.lists_session(row.privacy_tier) && !excluded_crew.contains(&row.session_id)
    });
    // Recomputed from the visible rows only: the plan the roster function built
    // was derived from every child, including the ones just omitted.
    let plan =
        biorouter::agents::orchestration_plan::snapshot(manager, &session_id, &visible.subagents)
            .await;
    Json(with_plan(visible, plan)).into_response()
}

/// The roster with only the rows `shows` admits, in their original order, and
/// with no plan: a plan derived from the unfiltered roster counts the omitted
/// rows, so it is dropped here and rebuilt by the caller from what remains.
pub(crate) fn visible_roster(
    roster: SessionSubagents,
    shows: impl Fn(&SubagentRow) -> bool,
) -> SessionSubagents {
    let SessionSubagents {
        parent_session_id,
        mut subagents,
        plan: _,
    } = roster;
    subagents.retain(|row| shows(row));
    SessionSubagents {
        parent_session_id,
        subagents,
        plan: None,
    }
}

/// Attach `plan` to the filtered roster, confining every phase's child ids to
/// the rows the roster still holds.
///
/// The plan records the ids of the children spawned into each phase when they
/// were spawned, so a child the caller may not see can still be named there. Its
/// id alone says that a run the caller could not open exists, which is what
/// omitting its row is for.
pub(crate) fn with_plan(
    mut roster: SessionSubagents,
    plan: Option<OrchestrationSnapshot>,
) -> SessionSubagents {
    roster.plan = plan.map(|mut plan| {
        let visible: HashSet<&str> = roster
            .subagents
            .iter()
            .map(|row| row.session_id.as_str())
            .collect();
        for phase in &mut plan.phases {
            phase.agent_ids.retain(|id| visible.contains(id.as_str()));
        }
        plan
    });
    roster
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/sessions/{session_id}/subagents",
            get(get_session_subagents),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use biorouter::agents::orchestration_plan::{
        PhaseCounts, PhaseKind, PhaseSnapshot, PhaseStatus,
    };
    use biorouter::agents::subagent_roster::SubagentRunStatus;
    use biorouter::model::ModelConfig;
    use biorouter::privacy::SessionClassification;
    use biorouter::session::SessionType;
    use serial_test::serial;
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn row(id: &str, tier: SessionClassification) -> SubagentRow {
        let now = chrono::Utc::now();
        SubagentRow {
            session_id: id.to_string(),
            name: format!("child {id}"),
            role: None,
            phase_id: Some("research".to_string()),
            status: SubagentRunStatus::Completed,
            loop_phase: None,
            quiet_seconds: None,
            created_at: now,
            updated_at: now,
            ended_at: Some(now),
            elapsed_seconds: Some(3),
            provider_name: None,
            model_name: None,
            extensions: Vec::new(),
            skills: Vec::new(),
            knowledge_bases: Vec::new(),
            privacy_tier: tier,
            total_tokens: None,
            message_count: 2,
            tool_call_count: 1,
            result_line: Some(format!("result of {id}")),
        }
    }

    fn phase(agent_ids: &[&str]) -> PhaseSnapshot {
        PhaseSnapshot {
            id: "research".to_string(),
            title: "Research".to_string(),
            goal: None,
            kind: PhaseKind::Research,
            status: PhaseStatus::InProgress,
            depends_on: Vec::new(),
            agent_ids: agent_ids.iter().map(|id| id.to_string()).collect(),
            counts: PhaseCounts::default(),
            handoff: None,
            started_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn the_filter_keeps_order_and_drops_the_stale_plan() {
        let roster = SessionSubagents {
            parent_session_id: "p".to_string(),
            subagents: vec![
                row("a", SessionClassification::Public),
                row("b", SessionClassification::Private),
                row("c", SessionClassification::Public),
            ],
            plan: Some(OrchestrationSnapshot {
                title: "Plan".to_string(),
                goal: None,
                revision: 1,
                phases: vec![phase(&["a", "b", "c"])],
            }),
        };
        let visible = visible_roster(roster, |row| {
            row.privacy_tier == SessionClassification::Public
        });
        assert_eq!(
            visible
                .subagents
                .iter()
                .map(|row| row.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        assert!(
            visible.plan.is_none(),
            "a plan derived from the unfiltered roster must not survive the filter"
        );
    }

    #[test]
    fn a_phase_never_names_a_child_the_roster_omitted() {
        let roster = SessionSubagents {
            parent_session_id: "p".to_string(),
            subagents: vec![row("a", SessionClassification::Public)],
            plan: None,
        };
        let plan = OrchestrationSnapshot {
            title: "Plan".to_string(),
            goal: None,
            revision: 3,
            phases: vec![phase(&["a", "hidden"]), phase(&["hidden"])],
        };
        let out = with_plan(roster, Some(plan));
        let plan = out.plan.expect("the plan is attached");
        assert_eq!(plan.phases[0].agent_ids, vec!["a".to_string()]);
        assert!(plan.phases[1].agent_ids.is_empty());
        assert_eq!(plan.revision, 3, "nothing else about the plan changes");

        let none = with_plan(
            SessionSubagents {
                parent_session_id: "p".to_string(),
                subagents: Vec::new(),
                plan: None,
            },
            None,
        );
        assert!(none.plan.is_none());
    }

    // ─── Over HTTP, against the binary's one session store ───

    /// A chat seeded here, deleted when the test's scope ends, including on a
    /// panic. `block_in_place` needs the multi-threaded runtime every test below
    /// asks for.
    struct Seeded {
        state: Arc<AppState>,
        id: String,
    }

    impl Drop for Seeded {
        fn drop(&mut self) {
            let state = self.state.clone();
            let id = std::mem::take(&mut self.id);
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    if let Err(e) = state.session_manager().delete_session(&id).await {
                        eprintln!("could not clean up the seeded chat {id}: {e}");
                    }
                })
            });
        }
    }

    async fn make_private(state: &Arc<AppState>, id: &str) {
        state
            .session_manager()
            .update(id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
    }

    async fn seed_parent(state: &Arc<AppState>, tier: SessionClassification) -> Seeded {
        let session = state
            .session_manager()
            .create_session(
                PathBuf::from("/tmp/session_subagents_route"),
                "Subagents route parent (test fixture)".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        if tier == SessionClassification::Private {
            make_private(state, &session.id).await;
        }
        Seeded {
            state: state.clone(),
            id: session.id,
        }
    }

    async fn seed_child(
        state: &Arc<AppState>,
        parent: &str,
        label: &str,
        tier: SessionClassification,
    ) -> Seeded {
        let manager = state.session_manager();
        let session = manager
            .create_session(
                PathBuf::from("/tmp/session_subagents_route"),
                format!("Subagent: {label}"),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        manager
            .update(&session.id)
            .parent_session_id(Some(parent.to_string()))
            .apply()
            .await
            .unwrap();
        if tier == SessionClassification::Private {
            make_private(state, &session.id).await;
        }
        Seeded {
            state: state.clone(),
            id: session.id,
        }
    }

    async fn get(
        state: Arc<AppState>,
        session_id: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        // The served tree, so a route registered on the wrong router is measured.
        let app = crate::routes::configure(state, "subagents-route-secret".to_string());
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{session_id}/subagents"));
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let res = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn ids(body: &str) -> Vec<String> {
        let value: serde_json::Value = serde_json::from_str(body).unwrap();
        value["subagents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["sessionId"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_answer_is_the_roster_shape_in_camel_case() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let parent = seed_parent(&state, SessionClassification::Public).await;
        let child = seed_child(&state, &parent.id, "shape", SessionClassification::Public).await;

        let (status, body) = get(state.clone(), &parent.id, &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["parentSessionId"], parent.id.as_str());
        assert!(value["subagents"].is_array());
        assert!(
            value.get("plan").is_some(),
            "`plan` is always present, null when no plan was declared: {body}"
        );
        // Exactly this parent's one child, and nothing else on the machine.
        assert_eq!(ids(&body), vec![child.id.clone()], "{body}");
        let row = &value["subagents"][0];
        for field in [
            "sessionId",
            "name",
            "status",
            "createdAt",
            "updatedAt",
            "extensions",
            "skills",
            "knowledgeBases",
            "privacyTier",
            "messageCount",
            "toolCallCount",
        ] {
            assert!(row.get(field).is_some(), "row has no `{field}`: {row}");
        }
        assert_eq!(
            row["name"], "shape",
            "the name is shown without its `Subagent: ` prefix: {row}"
        );
        assert!(
            row["status"].is_string(),
            "the status is a snake_case word: {row}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn an_unknown_session_is_refused_to_an_unproven_caller_and_404_to_the_user() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        // Syntactically a session id, and not a row on this machine.
        let absent = "29990101_515151";

        // Unreadable reads as private to a caller that proves nothing, so the
        // answer cannot be used to ask whether an id exists.
        let (status, body) = get(state.clone(), absent, &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(
            body,
            crate::routes::session_reach::SESSION_OUT_OF_REACH,
            "the refusal is not the reach gate's exact text"
        );

        let (status, _) = get(
            state.clone(),
            absent,
            &[("X-User-Action", TEST_USER_ACTION_KEY)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // A valid URI segment that is not a session id: refused before the gate
        // reads anything.
        let (status, _) = get(state.clone(), "not.a.session.id", &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_public_caller_cannot_list_the_subagents_of_a_private_chat() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let parent = seed_parent(&state, SessionClassification::Private).await;
        let _child = seed_child(
            &state,
            &parent.id,
            "private parent",
            SessionClassification::Private,
        )
        .await;

        let (status, body) = get(state.clone(), &parent.id, &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, crate::routes::session_reach::SESSION_OUT_OF_REACH);

        // The person at the keyboard, and a program stating a private model,
        // both still reach it.
        for credential in [
            ("X-User-Action", TEST_USER_ACTION_KEY),
            (
                crate::routes::session_reach::CALLER_PROVIDER_HEADER,
                "versa_azure",
            ),
        ] {
            let (status, body) = get(state.clone(), &parent.id, &[credential]).await;
            assert_eq!(status, StatusCode::OK, "{credential:?}: {body}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_private_child_of_a_public_chat_is_omitted_and_not_counted() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let parent = seed_parent(&state, SessionClassification::Public).await;
        let public_child = seed_child(
            &state,
            &parent.id,
            "public one",
            SessionClassification::Public,
        )
        .await;
        let private_child = seed_child(
            &state,
            &parent.id,
            "private one",
            SessionClassification::Private,
        )
        .await;

        // What the in-process roster holds, unfiltered. Without both rows here
        // the omission below would pass against an empty roster and prove
        // nothing about the filter.
        let unfiltered = biorouter::agents::subagent_roster::session_subagents(
            state.session_manager(),
            &parent.id,
        )
        .await
        .unwrap();
        let unfiltered_ids: Vec<&str> = unfiltered
            .subagents
            .iter()
            .map(|row| row.session_id.as_str())
            .collect();
        assert!(
            unfiltered_ids.contains(&private_child.id.as_str())
                && unfiltered_ids.contains(&public_child.id.as_str()),
            "the roster did not list both seeded children, so this test measures nothing: \
             {unfiltered_ids:?}"
        );

        let (status, body) = get(state.clone(), &parent.id, &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let listed = ids(&body);
        assert!(
            !listed.contains(&private_child.id),
            "a secret-only caller was listed a private subagent: {body}"
        );
        assert!(
            listed.contains(&public_child.id),
            "the public child was omitted too, so the filter refuses everything: {body}"
        );
        assert!(
            !body.contains("private one"),
            "the private child's name reached the body: {body}"
        );
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        if let Some(phases) = value["plan"]["phases"].as_array() {
            for phase in phases {
                let named = phase["agentIds"].as_array().unwrap();
                assert!(
                    !named.iter().any(|id| id == private_child.id.as_str()),
                    "a phase named the private child: {phase}"
                );
                let total = phase["counts"]["total"].as_u64().unwrap() as usize;
                assert!(
                    total <= listed.len(),
                    "a phase counted a child the caller was not listed: {phase}"
                );
            }
        }

        // The user sees both, which proves the omission above is the filter and
        // not an empty roster.
        let (status, body) = get(
            state.clone(),
            &parent.id,
            &[("X-User-Action", TEST_USER_ACTION_KEY)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let listed = ids(&body);
        assert!(listed.contains(&private_child.id), "{body}");
        assert!(listed.contains(&public_child.id), "{body}");
    }
}
