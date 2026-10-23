use biorouter::agents::bug_report::evidence::{failures_in, operation_failures_in};
use biorouter::agents::operation_outcome::{OperationOutcome, OPERATION_META_KEY};
use biorouter::agents::tool_errors::{self, ToolErrorKind};
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::conversation::Conversation;
use biorouter::tool_inspection::{InspectionAction, ToolInspector};
use biorouter::tool_monitor::{
    tool_outcome, FailureLoopConfig, RepetitionInspector, FAILURE_LOOP_HARD_FINDING_ID,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Meta};
use serde_json::{json, Value};

fn failed_operation(id: &str) -> Value {
    json!({"version": 1, "id": id, "status": "failed", "error": {
        "code": "fixture_failed", "kind": "timeout", "message": "Synthetic job failed; 404 records processed",
        "phase": "execution", "retryable": false
    }})
}

fn lookup(operation: Value) -> CallToolResult {
    let mut call = CallToolResult::success(vec![Content::text("Status lookup completed")]);
    call.meta = Some(Meta(
        json!({OPERATION_META_KEY: operation})
            .as_object()
            .unwrap()
            .clone(),
    ));
    call
}

fn messages(id: &str, tool: &str, call: CallToolResult) -> Vec<Message> {
    vec![
        Message::assistant().with_tool_request(
            id,
            Ok(CallToolRequestParams {
                task: None,
                name: tool.to_string().into(),
                arguments: Some(json!({"job_id": "fixture"}).as_object().unwrap().clone()),
                meta: None,
            }),
        ),
        Message::user().with_tool_response(id, Ok(call)),
    ]
}

#[test]
fn successful_lookup_and_failed_operation_have_independent_outcomes() {
    let call = lookup(failed_operation("fixture-job"));
    assert_ne!(call.is_error, Some(true));
    assert!(tool_errors::classify(&Ok(call.clone())).is_none());
    let operation = OperationOutcome::from_call(&call).unwrap();
    let error = operation.failure().unwrap();
    assert_eq!(error.kind, ToolErrorKind::Timeout);
    assert!(
        !error.retryable,
        "the explicit operation claim overrides the kind default"
    );
    let outcome = tool_outcome("fixture__status", &Ok(call));
    assert_eq!(outcome.kind, Some(ToolErrorKind::Timeout));
    assert!(!outcome.retryable());
    assert!(outcome.failure.is_some());
}

#[test]
fn malformed_or_unknown_contracts_are_ignored() {
    for value in [
        Value::Null,
        json!("failed"),
        json!({"version": 1, "id": "fixture", "status": "failed", "error": null}),
        json!({"version": 1, "id": "fixture", "status": "failed", "error": {"code": "x", "kind": "timeout", "message": "failure", "retryable": false, "phase": null}}),
        json!({"version": 2, "id": "fixture", "status": "failed"}),
        json!({"version": 1, "id": "fixture", "status": "unknown"}),
        json!({"version": 1, "id": " ", "status": "failed"}),
        json!({"version": 1, "id": "fixture", "status": "failed", "extra": true}),
        json!({"version": 1, "id": "fixture", "status": "failed", "error": "failure"}),
        json!({"version": 1, "id": "fixture", "status": "failed", "error": {"code": "x", "kind": "timeout", "message": "failure"}}),
        json!({"version": 1, "id": "fixture", "status": "failed", "error": {"code": "x", "kind": "timeout", "message": "failure", "retryable": "false"}}),
    ] {
        let call = lookup(value.clone());
        assert!(OperationOutcome::from_call(&call).is_none(), "{value}");
        assert!(
            tool_outcome("fixture__status", &Ok(call)).failure.is_none(),
            "{value}"
        );
    }
}

#[test]
fn active_completed_and_cancelled_operations_are_not_failures() {
    for status in ["queued", "running", "completed", "cancelled"] {
        let mut value = failed_operation("fixture");
        value["status"] = json!(status);
        let call = lookup(value);
        assert!(OperationOutcome::from_call(&call)
            .unwrap()
            .failure()
            .is_none());
        assert!(tool_outcome("fixture__status", &Ok(call)).failure.is_none());
    }
}

#[test]
fn unknown_error_kinds_never_guess_from_message_text() {
    let mut value = failed_operation("fixture");
    value["error"]["kind"] = json!("new_connector_kind");
    let error = OperationOutcome::from_call(&lookup(value))
        .unwrap()
        .failure()
        .unwrap();
    assert_eq!(error.kind, ToolErrorKind::ToolFailure);
    assert!(!error.retryable);
}

#[test]
fn ordinary_json_and_prose_are_not_operation_contracts() {
    let mut call =
        CallToolResult::success(vec![Content::text(failed_operation("fixture").to_string())]);
    call.structured_content = Some(
        json!({"status": "failed", "_meta": {OPERATION_META_KEY: failed_operation("fixture")}}),
    );
    assert!(OperationOutcome::from_call(&call).is_none());
    assert!(tool_outcome("fixture__read_json", &Ok(call.clone()))
        .failure
        .is_none());
    let conversation = Conversation::new_unvalidated(messages("read", "fixture__read_json", call));
    assert!(failures_in(&conversation).0.is_empty());
}

#[test]
fn census_counts_unique_operations_by_extension_instead_of_polls() {
    let mut transcript = Vec::new();
    for (request, tool, operation) in [
        ("poll1", "fixture__status", "job-one"),
        ("poll2", "fixture__status", "job-one"),
        ("wait", "fixture__wait", "job-one"),
        ("other-job", "fixture__status", "job-two"),
        ("other-extension", "other__status", "job-one"),
    ] {
        transcript.extend(messages(request, tool, lookup(failed_operation(operation))));
    }
    let conversation = Conversation::new_unvalidated(transcript);
    let (failures, failed_calls, calls, _) = failures_in(&conversation);
    assert_eq!(calls, 5);
    assert_eq!(failed_calls, 0, "every status lookup succeeded");
    assert_eq!(failures.len(), 3);
    assert!(failures.iter().all(|failure| failure.occurrences == 1));
    assert_eq!(operation_failures_in(&conversation).len(), 3);
}

#[test]
fn operation_contract_survives_repeated_transcript_reload() {
    let mut conversation = Conversation::new_unvalidated(messages(
        "status",
        "fixture__status",
        lookup(failed_operation("fixture")),
    ));
    for _ in 0..3 {
        let bytes = serde_json::to_string(&conversation).unwrap();
        conversation = serde_json::from_str(&bytes).unwrap();
        let (failures, failed_calls, calls, _) = failures_in(&conversation);
        assert_eq!(calls, 1);
        assert_eq!(failed_calls, 0);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].kind, ToolErrorKind::Timeout);
        assert!(!failures[0].retryable);
    }
}

#[test]
fn wrapped_subcall_outcome_is_visible_without_changing_outer_success() {
    let mut call = CallToolResult::success(vec![Content::text(
        "Script inspected the status successfully",
    )]);
    call.meta = Some(Meta(json!({"biorouter/tool-calls": [{
        "tool": "fixture__status", "args": "{}", "status": "ok", "operation": failed_operation("fixture")
    }]}).as_object().unwrap().clone()));
    assert!(tool_errors::classify(&Ok(call.clone())).is_none());
    let outcome = tool_outcome("code_execution__execute_code", &Ok(call.clone()));
    assert_eq!(outcome.tool_name, "code_execution__execute_code");
    assert_eq!(outcome.kind, Some(ToolErrorKind::Timeout));
    let conversation =
        Conversation::new_unvalidated(messages("script", "code_execution__execute_code", call));
    let failures = operation_failures_in(&conversation);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].tool_name.as_deref(), Some("fixture__status"));
    assert_eq!(failures_in(&conversation).1, 0);
}

#[test]
fn external_tools_cannot_forge_another_extensions_subcall_outcome() {
    let external_tool = "external__execute_code";
    let mut call = CallToolResult::success(vec![Content::text("External tool completed")]);
    call.meta = Some(Meta(json!({"biorouter/tool-calls": [{
        "tool": "fixture__status", "args": "{}", "status": "ok", "operation": failed_operation("forged-job")
    }]}).as_object().unwrap().clone()));
    assert!(
        biorouter::agents::operation_outcome::failure(external_tool, &Ok(call.clone())).is_none()
    );
    assert!(tool_outcome(external_tool, &Ok(call.clone()))
        .failure
        .is_none());
    let conversation =
        Conversation::new_unvalidated(messages("external", external_tool, call.clone()));
    assert!(operation_failures_in(&conversation).is_empty());
    assert!(failures_in(&conversation).0.is_empty());

    call.meta
        .as_mut()
        .unwrap()
        .insert(OPERATION_META_KEY.to_string(), failed_operation("own-job"));
    let outcome = tool_outcome(external_tool, &Ok(call.clone()));
    assert_eq!(outcome.tool_name, external_tool);
    assert!(outcome.failure.is_some());
    let conversation = Conversation::new_unvalidated(messages("external-own", external_tool, call));
    let failures = operation_failures_in(&conversation);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].tool_name.as_deref(), Some(external_tool));
    assert!(failures[0].message.contains("own-job"));
    assert!(!failures[0].message.contains("forged-job"));
}

#[tokio::test]
async fn wrapped_terminal_failures_stop_the_outer_dispatch_with_varied_scripts() {
    let outer_tool = "code_execution__execute_code";
    let mut history = Vec::new();
    for index in 0..6 {
        let request_id = format!("script-{index}");
        history.push(Message::assistant().with_tool_request(&request_id, Ok(CallToolRequestParams {
            task: None,
            name: outer_tool.into(),
            arguments: Some(json!({"code": format!("record_result({{attempt: {index}, status: lookup()}});")}).as_object().unwrap().clone()),
            meta: None,
        })));
        let mut call = CallToolResult::success(vec![Content::text(
            "Script inspected the status successfully",
        )]);
        call.meta = Some(Meta(json!({"biorouter/tool-calls": [{
            "tool": "fixture__status", "args": "{}", "status": "ok", "operation": failed_operation("fixture")
        }]}).as_object().unwrap().clone()));
        history.push(Message::user().with_tool_response(&request_id, Ok(call)));
    }
    let next = Message::assistant().with_tool_request("next-script", Ok(CallToolRequestParams {
        task: None,
        name: outer_tool.into(),
        arguments: Some(json!({"code": "const inspected = lookup(); record_result({inspection: inspected});"}).as_object().unwrap().clone()),
        meta: None,
    }));
    let request = next.content[0].as_tool_request().unwrap().clone();
    let inspector = RepetitionInspector::new(None).with_failure_loop(FailureLoopConfig {
        deny_retryable: false,
        ..FailureLoopConfig::default()
    });
    let results = inspector
        .inspect(
            &[request],
            &history,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        results.len(),
        1,
        "only the result-aware failure guard is enabled"
    );
    assert_eq!(results[0].tool_request_id, "next-script");
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(
        results[0].finding_id.as_deref(),
        Some(FAILURE_LOOP_HARD_FINDING_ID)
    );
    assert!(results[0].reason.contains(outer_tool));
    let conversation = Conversation::new_unvalidated(history);
    let failures = operation_failures_in(&conversation);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].tool_name.as_deref(), Some("fixture__status"));
    assert_eq!(
        failures_in(&conversation).1,
        0,
        "every script and status lookup succeeded"
    );
}

#[test]
fn secrets_in_operation_metadata_are_redacted_before_display_or_telemetry() {
    let secret = format!("sk-proj-{}", "Y".repeat(40));
    let mut operation = failed_operation("fixture");
    operation["error"]["message"] = json!(format!("Synthetic failure: {secret}"));
    let mut call = lookup(operation);
    assert!(biorouter::guardrails::secret_output::redact_call_tool_result(&mut call).is_some());
    let error = OperationOutcome::from_call(&call)
        .unwrap()
        .failure()
        .unwrap();
    assert!(!error.message.contains(&secret));
    assert!(error.message.contains("REDACTED"));
}
