use biorouter::agents::bug_report::evidence::{
    failures_in, nested_failures_in, operation_failures_in,
};
use biorouter::agents::tool_errors::{self, ToolErrorKind};
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::conversation::Conversation;
use biorouter::tool_inspection::{InspectionAction, ToolInspector};
use biorouter::tool_monitor::{
    failure_loop_nudge, tool_outcome, tool_outcomes_since_last_progress, FailureLoopConfig,
    RepetitionInspector, FAILURE_LOOP_HARD_FINDING_ID,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Meta};
use serde_json::{json, Value};

fn nested_result(records: Value) -> CallToolResult {
    let mut result = CallToolResult::success(vec![Content::text(
        "Result: {\"caught\":true,\"message\":\"nested failure was handled\"}",
    )]);
    result.meta = Some(Meta(
        json!({"biorouter/tool-calls": records})
            .as_object()
            .unwrap()
            .clone(),
    ));
    result
}

fn typed_failure() -> Value {
    json!({
        "tool": "fixture__query_job_status", "args": "{\"job_id\":\"missing\"}",
        "status": "error", "error": "404 permission denied; retry later",
        "failure": {"kind": "timeout", "retryable": false}
    })
}

fn request(id: &str, name: &str) -> Message {
    Message::assistant().with_tool_request(
        id,
        Ok(CallToolRequestParams {
            task: None,
            name: name.to_string().into(),
            arguments: Some(json!({"code": format!("try {{ status({id:?}) }} catch (e) {{ record_result(e) }}")}).as_object().unwrap().clone()),
            meta: None,
        }),
    )
}

fn conversation(name: &str, result: CallToolResult) -> Conversation {
    let original = Conversation::new_unvalidated(vec![
        request("script", name),
        Message::user().with_tool_response("script", Ok(result)),
    ]);
    serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap()
}

#[test]
fn caught_failure_survives_reload_without_changing_outer_or_operation_counts() {
    for name in ["code_execution__execute_code", "execute_code"] {
        let result = nested_result(json!([typed_failure()]));
        assert_ne!(result.is_error, Some(true));
        assert!(tool_errors::classify(&Ok(result.clone())).is_none());
        let transcript = conversation(name, result.clone());
        let (failures, failed_outer, calls, _) = failures_in(&transcript);
        assert_eq!(calls, 1);
        assert_eq!(failed_outer, 0);
        assert!(operation_failures_in(&transcript).is_empty());
        assert_eq!(nested_failures_in(&transcript).len(), 1);
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].tool_name.as_deref(),
            Some("fixture__query_job_status")
        );
        assert_eq!(failures[0].kind, ToolErrorKind::Timeout);
        assert!(!failures[0].retryable);
        let outcome = tool_outcome(name, &Ok(result));
        assert_eq!(outcome.tool_name, name);
        assert_eq!(outcome.kind, Some(ToolErrorKind::Timeout));
        assert!(!outcome.retryable());
        assert!(outcome.failure.is_some());
    }
}

#[test]
fn old_persisted_telemetry_is_recognized_only_from_the_actual_builtin_call() {
    let result = nested_result(json!([
        {"tool":"fixture__query_job_capacity","args":"{}","status":"ok","result_bytes":57},
        {"tool":"fixture__query_job_status","args":"{}","status":"error","error":"Unknown job ID."}
    ]));
    let failures = nested_failures_in(&conversation(
        "code_execution__execute_code",
        result.clone(),
    ));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].message, "Unknown job ID.");
    for external in ["fixture__execute_code", "external__read_json"] {
        let transcript = conversation(external, result.clone());
        assert!(failures_in(&transcript).0.is_empty());
        assert!(tool_outcome(external, &Ok(result.clone()))
            .failure
            .is_none());
    }
}

#[test]
fn uncaught_errors_and_failed_job_operations_are_not_counted_again_as_caught_calls() {
    let mut uncaught = nested_result(json!([typed_failure()]));
    uncaught.is_error = Some(true);
    let transcript = conversation("code_execution__execute_code", uncaught);
    assert_eq!(failures_in(&transcript).1, 1);
    assert_eq!(failures_in(&transcript).0.len(), 1);
    assert!(nested_failures_in(&transcript).is_empty());

    for status in ["ok", "error"] {
        let mut record = typed_failure();
        record["status"] = json!(status);
        record["operation"] = json!({"version":1,"id":"fixture-job","status":"failed"});
        let transcript = conversation(
            "code_execution__execute_code",
            nested_result(json!([record])),
        );
        assert_eq!(operation_failures_in(&transcript).len(), 1);
        assert!(nested_failures_in(&transcript).is_empty());
        assert_eq!(failures_in(&transcript).0.len(), 1);
        assert_eq!(failures_in(&transcript).1, 0);
    }
}

#[test]
fn success_prose_and_malformed_typed_provenance_cannot_invent_a_retryable_failure() {
    let mut success = typed_failure();
    success["status"] = json!("ok");
    assert!(nested_failures_in(&conversation(
        "code_execution__execute_code",
        nested_result(json!([success]))
    ))
    .is_empty());
    for provenance in [
        json!({"kind":"new_kind","retryable":true}),
        json!({"kind":"timeout","retryable":"false"}),
        json!({"kind":"timeout","retryable":true,"untrusted":true}),
    ] {
        let mut record = typed_failure();
        record["failure"] = provenance;
        let outcome = tool_outcome(
            "code_execution__execute_code",
            &Ok(nested_result(json!([record]))),
        );
        assert_eq!(outcome.kind, Some(ToolErrorKind::ToolFailure));
        assert!(!outcome.retryable());
    }
}

#[tokio::test]
async fn repeated_caught_failures_warn_then_stop_the_outer_dispatch_despite_varied_scripts() {
    let name = "code_execution__execute_code";
    let config = FailureLoopConfig {
        deny_retryable: false,
        ..FailureLoopConfig::default()
    };
    let inspector = RepetitionInspector::new(None).with_failure_loop(config.clone());
    let mut history = Vec::new();
    for index in 0..6 {
        let id = format!("attempt-{index}");
        history.push(request(&id, name));
        history.push(
            Message::user().with_tool_response(&id, Ok(nested_result(json!([typed_failure()])))),
        );
        if index == 2 {
            let outcomes = tool_outcomes_since_last_progress(&history, &config);
            assert!(failure_loop_nudge(&config, &outcomes, name).is_some());
            let next = request("recovery", name);
            let inspected = inspector
                .inspect(
                    &[next.content[0].as_tool_request().unwrap().clone()],
                    &history,
                    BioRouterMode::Approve,
                    &biorouter::session::Session::default(),
                )
                .await
                .unwrap();
            assert!(inspected
                .iter()
                .all(|result| result.action != InspectionAction::Deny));
        }
    }
    let next = request("after-recovery", name);
    let inspected = inspector
        .inspect(
            &[next.content[0].as_tool_request().unwrap().clone()],
            &history,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .unwrap();
    assert!(inspected
        .iter()
        .any(|result| result.action == InspectionAction::Deny
            && result.finding_id.as_deref() == Some(FAILURE_LOOP_HARD_FINDING_ID)));
    let transcript = Conversation::new_unvalidated(history);
    assert_eq!(failures_in(&transcript).1, 0);
    assert_eq!(nested_failures_in(&transcript)[0].occurrences, 6);
}
