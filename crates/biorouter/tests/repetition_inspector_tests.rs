use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::tool_inspection::{
    collect_warning_reasons, frame_loop_warnings, InspectionAction, ToolInspector,
};
use biorouter::tool_monitor::{
    RepetitionInspector, REPETITION_HARD_FINDING_ID, REPETITION_SOFT_FINDING_ID,
};
use rmcp::model::CallToolRequestParams;
use rmcp::object;
use serde_json::json;

fn tool_call(name: &str, id: i32) -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        meta: None,
        name: name.to_string().into(),
        arguments: Some(object!({"id": id})),
    }
}

fn tool_request(id: &str, call: CallToolRequestParams) -> ToolRequest {
    let message = Message::assistant().with_tool_request(id, Ok(call));
    message
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("request message should contain a tool request")
        .clone()
}

// This test targets the production RepetitionInspector::inspect path.
// It verifies that within a single batch of tool requests:
// - each exact tool + canonical-arguments signature has its own count
// - the (max_repetitions + 1)th occurrence of one signature is denied
// - a different argument signature has an independent count
#[tokio::test]
async fn test_repetition_inspector_counts_each_exact_signature_independently() {
    // Allow at most 2 occurrences of either exact signature.
    let inspector = RepetitionInspector::new(Some(2));

    let call_v1 = tool_call("fetch_user", 123);
    let call_v2 = tool_call("fetch_user", 456);

    let requests = vec![
        tool_request("call_1", call_v1.clone()), // 1st identical → allowed
        tool_request("call_2", call_v1.clone()), // 2nd identical → allowed (at limit)
        tool_request("call_3", call_v1),         // 3rd identical → denied
        tool_request("call_4", call_v2.clone()), // different signature → allowed
        tool_request("call_5", call_v2.clone()), // 2nd with new params → allowed (at limit)
        tool_request("call_6", call_v2),         // 3rd with new params → denied
    ];

    let results = inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    // Only the calls that exceed their signature's limit are denied.
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].tool_request_id, "call_3");
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(results[0].finding_id.as_deref(), Some("REP-001"));
    assert_eq!(results[1].tool_request_id, "call_6");
    assert_eq!(results[1].action, InspectionAction::Deny);
    assert_eq!(results[1].finding_id.as_deref(), Some("REP-001"));
}

#[tokio::test]
async fn test_repetition_inspector_denies_current_request_after_history_repeats() {
    let inspector = RepetitionInspector::new(Some(2));
    let call = tool_call("fetch_user", 123);
    let messages = vec![
        Message::assistant().with_tool_request("call_1", Ok(call.clone())),
        Message::user().with_tool_response("call_1", Ok(tool_ok("tool response"))),
        Message::assistant().with_tool_request("call_2", Ok(call.clone())),
        Message::user().with_tool_response("call_2", Ok(tool_ok("tool response"))),
    ];
    let request_message = Message::assistant().with_tool_request("call_3", Ok(call));
    let request = request_message
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("request message should contain a tool request")
        .clone();

    let results = inspector
        .inspect(
            &[request],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(results[0].finding_id.as_deref(), Some("REP-001"));
}

#[tokio::test]
async fn test_repetition_inspector_allows_changed_arguments_after_history_repeats() {
    let inspector = RepetitionInspector::new(Some(2));
    let prior_call = tool_call("fetch_user", 123);
    let messages = vec![
        Message::assistant().with_tool_request("call_1", Ok(prior_call.clone())),
        Message::user().with_tool_response("call_1", Ok(tool_ok("tool response"))),
        Message::assistant().with_tool_request("call_2", Ok(prior_call)),
        Message::user().with_tool_response("call_2", Ok(tool_ok("tool response"))),
    ];
    let request_message =
        Message::assistant().with_tool_request("call_3", Ok(tool_call("fetch_user", 456)));
    let request = request_message
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("request message should contain a tool request")
        .clone();

    let results = inspector
        .inspect(
            &[request],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert!(results.is_empty());
}

/// Exact signatures are counted across intervening calls within one user turn.
/// Alternating `A, B, A, B` must not hide the third occurrence of `A`.
#[tokio::test]
async fn test_interleaved_exact_signature_is_detected() {
    let inspector = RepetitionInspector::new(Some(2));
    let looping = tool_call("ui_describe", 1);
    let filler = tool_call("ui_render", 99);
    let messages = vec![
        Message::assistant().with_tool_request("call_1", Ok(looping.clone())),
        Message::user().with_tool_response("call_1", Ok(tool_ok("done"))),
        Message::assistant().with_tool_request("call_2", Ok(filler.clone())),
        Message::user().with_tool_response("call_2", Ok(tool_ok("done"))),
        Message::assistant().with_tool_request("call_3", Ok(looping.clone())),
        Message::user().with_tool_response("call_3", Ok(tool_ok("done"))),
        Message::assistant().with_tool_request("call_4", Ok(filler)),
        Message::user().with_tool_response("call_4", Ok(tool_ok("done"))),
    ];

    let results = inspector
        .inspect(
            &[tool_request("call_5", looping)],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tool_request_id, "call_5");
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(results[0].inspector_name, "repetition");
}

/// Calls to one tool with different canonical arguments are different
/// signatures and therefore represent progress rather than repetition.
#[tokio::test]
async fn test_distinct_argument_signatures_are_never_a_loop() {
    let inspector = RepetitionInspector::new(Some(2));
    let messages = vec![
        Message::assistant().with_tool_request("call_1", Ok(tool_call("fetch_user", 1))),
        Message::assistant().with_tool_request("call_2", Ok(tool_call("fetch_user", 2))),
        Message::assistant().with_tool_request("call_3", Ok(tool_call("fetch_user", 3))),
    ];

    let results = inspector
        .inspect(
            &[tool_request("call_4", tool_call("fetch_user", 4))],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert!(
        results.is_empty(),
        "different argument signatures must not share a count: {results:?}"
    );
}

/// The tool name is part of the signature: identical arguments sent to a
/// different tool must not inherit another tool's repetition count.
#[tokio::test]
async fn test_tool_name_is_part_of_the_exact_signature() {
    let inspector = RepetitionInspector::new(Some(2));
    let messages = vec![
        Message::assistant().with_tool_request("call_1", Ok(tool_call("fetch_user", 7))),
        Message::assistant().with_tool_request("call_2", Ok(tool_call("fetch_user", 7))),
    ];

    let results = inspector
        .inspect(
            &[tool_request("call_3", tool_call("update_user", 7))],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert!(
        results.is_empty(),
        "a different tool name must start a different signature: {results:?}"
    );
}

/// JSON object key order is not part of a tool call's meaning. Canonical
/// arguments with reversed insertion order must share the same signature.
#[tokio::test]
async fn test_canonical_argument_order_shares_one_signature() {
    let inspector = RepetitionInspector::new(Some(2));

    let mut first_args = serde_json::Map::new();
    first_args.insert("patient".to_string(), json!(7));
    first_args.insert("region".to_string(), json!("west"));
    let mut reversed_args = serde_json::Map::new();
    reversed_args.insert("region".to_string(), json!("west"));
    reversed_args.insert("patient".to_string(), json!(7));

    let first = call_with("fetch_user", serde_json::Value::Object(first_args));
    let reversed = call_with("fetch_user", serde_json::Value::Object(reversed_args));
    let messages = vec![
        Message::assistant().with_tool_request("call_1", Ok(first.clone())),
        Message::assistant().with_tool_request("call_2", Ok(reversed)),
    ];

    let results = inspector
        .inspect(
            &[tool_request("call_3", first)],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(results.len(), 1, "canonical key order must compare equal");
    assert_eq!(results[0].action, InspectionAction::Deny);
}

/// The newest genuine user message starts a fresh signature window. Tool
/// responses before or after it do not create additional resets.
#[tokio::test]
async fn test_exact_signature_count_resets_after_newest_user_message() {
    let inspector = RepetitionInspector::new(Some(2));
    let call = tool_call("fetch_user", 123);
    let messages = vec![
        Message::assistant().with_tool_request("old_1", Ok(call.clone())),
        Message::user().with_tool_response("old_1", Ok(tool_ok("done"))),
        Message::assistant().with_tool_request("old_2", Ok(call.clone())),
        Message::user().with_tool_response("old_2", Ok(tool_ok("done"))),
        Message::user().with_text("try that lookup again with the new context"),
        Message::assistant().with_tool_request("new_1", Ok(call.clone())),
        Message::user().with_tool_response("new_1", Ok(tool_ok("done"))),
    ];

    let results = inspector
        .inspect(
            &[
                tool_request("new_2", call.clone()),
                tool_request("new_3", call),
            ],
            &messages,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(
        results.len(),
        1,
        "only the third call in the new turn stops"
    );
    assert_eq!(results[0].tool_request_id, "new_3");
    assert_eq!(results[0].action, InspectionAction::Deny);
}

// BR-29: the guard is staged. The soft stage nudges the model (the call still
// runs); only a model that keeps repeating itself through the nudge is stopped.
#[tokio::test]
async fn test_staged_repetition_warns_before_it_stops() {
    // Warn on the 3rd occurrence of one signature, deny on the 5th.
    let inspector = RepetitionInspector::staged(3, 5);
    let call = tool_call("fetch_user", 123);

    let requests = vec![
        tool_request("call_1", call.clone()), // 1st → silent
        tool_request("call_2", call.clone()), // 2nd → silent
        tool_request("call_3", call.clone()), // 3rd → soft warning, still runs
        tool_request("call_4", call.clone()), // 4th → soft warning, still runs
        tool_request("call_5", call.clone()), // 5th → hard stop
        tool_request("call_6", call),         // 6th → hard stop
    ];

    let results = inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(results.len(), 4);

    assert_eq!(results[0].tool_request_id, "call_3");
    assert_eq!(results[0].action, InspectionAction::Warn);
    assert_eq!(
        results[0].finding_id.as_deref(),
        Some(REPETITION_SOFT_FINDING_ID)
    );
    assert_eq!(results[1].tool_request_id, "call_4");
    assert_eq!(results[1].action, InspectionAction::Warn);

    assert_eq!(results[2].tool_request_id, "call_5");
    assert_eq!(results[2].action, InspectionAction::Deny);
    assert_eq!(
        results[2].finding_id.as_deref(),
        Some(REPETITION_HARD_FINDING_ID)
    );
    assert_eq!(results[3].tool_request_id, "call_6");
    assert_eq!(results[3].action, InspectionAction::Deny);
}

// The soft warning names the tool, the count, and the approaching hard stop, so
// the model can act on it.
#[tokio::test]
async fn test_soft_warning_is_actionable_guidance() {
    let inspector = RepetitionInspector::staged(3, 5);
    let call = tool_call("fetch_user", 123);
    let requests = vec![
        tool_request("call_1", call.clone()),
        tool_request("call_2", call.clone()),
        tool_request("call_3", call),
    ];

    let results = inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(results.len(), 1);
    let reason = &results[0].reason;
    assert!(reason.contains("fetch_user"), "reason: {reason}");
    assert!(reason.contains('3'), "reason: {reason}");
    assert!(reason.contains('5'), "reason: {reason}");
    assert!(
        !reason.to_lowercase().contains("declined"),
        "a warning must not claim anyone declined: {reason}"
    );
}

// BR-29 core bug: on a repetition stop the model used to be handed
// DECLINED_RESPONSE ("The user has declined to run this tool"), which is false.
// The deny reason must state the real cause.
#[tokio::test]
async fn test_hard_stop_reason_is_honest_not_a_fake_user_decline() {
    let inspector = RepetitionInspector::staged(3, 5);
    let call = tool_call("fetch_user", 123);
    let requests: Vec<ToolRequest> = (1..=5)
        .map(|i| tool_request(&format!("call_{i}"), call.clone()))
        .collect();

    let results = inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    let deny = results
        .iter()
        .find(|result| result.action == InspectionAction::Deny)
        .expect("the 5th identical call should be denied");

    let reason = deny.reason.to_lowercase();
    assert!(
        !reason.contains("the user has declined"),
        "deny reason must not blame the user: {}",
        deny.reason
    );
    assert!(
        reason.contains("did not decline"),
        "deny reason should say the user did not decline: {}",
        deny.reason
    );
    assert!(
        reason.contains("repetition"),
        "deny reason should name the repetition guard: {}",
        deny.reason
    );
}

// A soft warning is advisory: it must not deny the call or force an approval
// prompt.
#[test]
fn test_warn_action_does_not_change_the_permission_verdict() {
    use biorouter::permission::permission_judge::PermissionCheckResult;
    use biorouter::tool_inspection::{apply_inspection_results_to_permissions, InspectionResult};

    let request = tool_request("call_1", tool_call("fetch_user", 123));
    let permission_result = PermissionCheckResult {
        approved: vec![request.clone()],
        needs_approval: vec![],
        denied: vec![],
    };

    let inspection_results = vec![InspectionResult {
        tool_request_id: request.id.clone(),
        action: InspectionAction::Warn,
        reason: "Repetition warning: you have called 'fetch_user' 3 times in a row.".to_string(),
        confidence: 1.0,
        inspector_name: "repetition".to_string(),
        finding_id: Some(REPETITION_SOFT_FINDING_ID.to_string()),
    }];

    let updated = apply_inspection_results_to_permissions(permission_result, &inspection_results);

    assert_eq!(updated.approved.len(), 1);
    assert!(updated.denied.is_empty());
    assert!(updated.needs_approval.is_empty());
}

// The warnings the agent injects into the model's context: de-duplicated,
// framed, and only from Warn results.
#[test]
fn test_collect_and_frame_warning_reasons() {
    use biorouter::tool_inspection::InspectionResult;

    let warn = |id: &str, reason: &str| InspectionResult {
        tool_request_id: id.to_string(),
        action: InspectionAction::Warn,
        reason: reason.to_string(),
        confidence: 1.0,
        inspector_name: "repetition".to_string(),
        finding_id: Some(REPETITION_SOFT_FINDING_ID.to_string()),
    };

    let results = vec![
        warn("call_1", "repeating 'fetch_user'"),
        warn("call_2", "repeating 'fetch_user'"), // duplicate reason → collapsed
        InspectionResult {
            tool_request_id: "call_3".to_string(),
            action: InspectionAction::Deny,
            reason: "hard stop".to_string(),
            confidence: 1.0,
            inspector_name: "repetition".to_string(),
            finding_id: Some(REPETITION_HARD_FINDING_ID.to_string()),
        },
    ];

    let reasons = collect_warning_reasons(&results);
    assert_eq!(reasons, vec!["repeating 'fetch_user'".to_string()]);

    let framed = frame_loop_warnings(&reasons);
    assert!(framed.contains("<biorouter-loop-guard>"));
    assert!(framed.contains("repeating 'fetch_user'"));
    assert!(!framed.contains("hard stop"));
}

// Degenerate config (soft >= hard) must not fire a warning that can never be
// acted on — it degrades to the old hard-stop-only behavior.
#[tokio::test]
async fn test_soft_stage_is_inert_when_thresholds_collapse() {
    let inspector = RepetitionInspector::staged(5, 5);
    let call = tool_call("fetch_user", 123);
    let requests: Vec<ToolRequest> = (1..=5)
        .map(|i| tool_request(&format!("call_{i}"), call.clone()))
        .collect();

    let results = inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tool_request_id, "call_5");
    assert_eq!(results[0].action, InspectionAction::Deny);
}

// ---------------------------------------------------------------------------
// BR-30: semantic / near-duplicate / oscillation loop detection
// ---------------------------------------------------------------------------

use biorouter::tool_monitor::{
    SemanticLoopConfig, REPETITION_NEAR_DUP_HARD_FINDING_ID, REPETITION_NEAR_DUP_SOFT_FINDING_ID,
    REPETITION_OSCILLATION_HARD_FINDING_ID, REPETITION_OSCILLATION_SOFT_FINDING_ID,
};

/// A tool call with arbitrary arguments (the `tool_call` helper above only
/// varies an integer `id`).
fn call_with(name: &str, args: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        meta: None,
        name: name.to_string().into(),
        arguments: args.as_object().cloned(),
    }
}

fn shell(command: &str) -> CallToolRequestParams {
    call_with("shell", json!({ "command": command }))
}

async fn inspect(
    inspector: &RepetitionInspector,
    requests: &[ToolRequest],
) -> Vec<biorouter::tool_inspection::InspectionResult> {
    inspector
        .inspect(
            requests,
            &[],
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed")
}

fn requests(calls: Vec<CallToolRequestParams>) -> Vec<ToolRequest> {
    calls
        .into_iter()
        .enumerate()
        .map(|(index, call)| tool_request(&format!("call_{}", index + 1), call))
        .collect()
}

fn completed_calls(calls: Vec<CallToolRequestParams>, result: CallToolResult) -> Vec<Message> {
    calls
        .into_iter()
        .enumerate()
        .flat_map(|(index, call)| {
            let id = format!("completed-{index}");
            [
                Message::assistant().with_tool_request(&id, Ok(call)),
                Message::user().with_tool_response(&id, Ok(result.clone())),
            ]
        })
        .collect()
}

// The BR-30 case the exact-signature guard misses entirely: the model keeps calling
// the same tool, nudging the arguments by a character each time.
#[tokio::test]
async fn test_near_duplicate_arg_tweaks_are_flagged() {
    let inspector = RepetitionInspector::staged(3, 5);

    let requests = requests(vec![
        shell("grep -rn 'assemble_turn_context' crates/biorouter/src/agents/"),
        shell("grep -rn 'assemble_turn_context' crates/biorouter/src/agents"),
        shell("grep -rn 'assemble_turn_context' crates/biorouter/src/agent"),
        shell("grep -rn 'assemble_turn_context' crates/biorouter/src/agen"),
    ]);

    let results = inspect(&inspector, &requests).await;

    assert_eq!(results.len(), 1, "only the 4th call trips the soft stage");
    assert_eq!(results[0].tool_request_id, "call_4");
    assert_eq!(results[0].action, InspectionAction::Warn);
    assert_eq!(
        results[0].finding_id.as_deref(),
        Some(REPETITION_NEAR_DUP_SOFT_FINDING_ID)
    );
    assert!(results[0].reason.contains("shell"), "{}", results[0].reason);
    assert!(
        !results[0].reason.to_lowercase().contains("declined"),
        "a warning must never claim anyone declined: {}",
        results[0].reason
    );
}

// The false-positive that would make this heuristic unusable: a model working
// through a list of distinct inputs is making progress, not looping.
#[tokio::test]
async fn test_iterating_over_distinct_inputs_is_not_a_loop() {
    let inspector = RepetitionInspector::staged(3, 5);

    let results = inspect(
        &inspector,
        &requests(vec![
            call_with("read_file", json!({"path": "crates/a.rs"})),
            call_with("read_file", json!({"path": "crates/b.rs"})),
            call_with("read_file", json!({"path": "crates/c.rs"})),
            call_with("read_file", json!({"path": "crates/d.rs"})),
            call_with("read_file", json!({"path": "crates/e.rs"})),
            call_with("read_file", json!({"path": "crates/f.rs"})),
        ]),
    )
    .await;

    assert!(
        results.is_empty(),
        "distinct targets must not read as repetition: {results:?}"
    );
}

// OpenHands' A/B/A/B heuristic: two calls alternating, neither making progress.
#[tokio::test]
async fn test_ab_ab_oscillation_is_flagged() {
    let inspector = RepetitionInspector::staged(3, 5);

    let results = inspect(
        &inspector,
        &requests(vec![
            call_with("read_file", json!({"path": "main.rs"})),
            shell("cargo build"),
            call_with("read_file", json!({"path": "main.rs"})),
            shell("cargo build"),
        ]),
    )
    .await;

    assert_eq!(results.len(), 1, "the 4th call closes the second cycle");
    assert_eq!(results[0].tool_request_id, "call_4");
    assert_eq!(results[0].action, InspectionAction::Warn);
    assert_eq!(
        results[0].finding_id.as_deref(),
        Some(REPETITION_OSCILLATION_SOFT_FINDING_ID)
    );
    assert!(
        results[0].reason.contains("alternate"),
        "{}",
        results[0].reason
    );
}

// The exact-signature guard (BR-29) owns plain repetition; BR-30 must not pile a
// second nudge onto the same call.
#[tokio::test]
async fn test_exact_repeats_get_exactly_one_verdict() {
    let inspector = RepetitionInspector::staged(3, 5);
    let call = shell("cargo test");

    let results = inspect(
        &inspector,
        &requests(vec![
            call.clone(),
            call.clone(),
            call.clone(),
            call.clone(),
            call.clone(),
            call,
        ]),
    )
    .await;

    assert_eq!(
        results.len(),
        4,
        "one verdict per repeated call: {results:?}"
    );
    for result in &results {
        let finding = result.finding_id.as_deref();
        assert!(
            finding == Some(REPETITION_SOFT_FINDING_ID)
                || finding == Some(REPETITION_HARD_FINDING_ID),
            "exact-signature repeats belong to the BR-29 guard, got {finding:?}"
        );
    }
}

// Pending batches can warn; semantic enforcement requires completed evidence.
#[tokio::test]
async fn test_semantic_hard_stops_require_completed_evidence() {
    let tweaks = vec![
        shell("cat /etc/hosts | grep -n biorouter-dev-host"),
        shell("cat /etc/hosts | grep -n biorouter-dev-hos"),
        shell("cat /etc/hosts | grep -n biorouter-dev-ho"),
        shell("cat /etc/hosts | grep -n biorouter-dev-h"),
    ];

    let default_results = inspect(
        &RepetitionInspector::staged(3, 5),
        &requests(tweaks.clone()),
    )
    .await;
    assert!(
        default_results
            .iter()
            .all(|result| result.action == InspectionAction::Warn),
        "a pending batch has no observed no-progress evidence: {default_results:?}"
    );

    let strict = RepetitionInspector::staged(3, 5).with_semantic(SemanticLoopConfig {
        near_dup_hard_stop: Some(4),
        ..SemanticLoopConfig::default()
    });
    let next = tweaks.last().unwrap().clone();
    let history = completed_calls(tweaks, tool_error("unchanged failure"));
    let strict_results = inspect_with_history(&strict, &requests(vec![next]), &history).await;

    assert_eq!(strict_results.len(), 1);
    assert_eq!(strict_results[0].action, InspectionAction::Deny);
    assert_eq!(
        strict_results[0].finding_id.as_deref(),
        Some(REPETITION_NEAR_DUP_HARD_FINDING_ID)
    );
    assert!(
        strict_results[0].reason.contains("did NOT decline"),
        "a heuristic stop must still be honest about its cause: {}",
        strict_results[0].reason
    );
}

#[tokio::test]
async fn test_configured_oscillation_hard_stop_uses_completed_results() {
    let cycle = vec![
        call_with("read_file", json!({"path": "main.rs"})),
        shell("cargo build"),
        call_with("read_file", json!({"path": "main.rs"})),
        shell("cargo build"),
    ];

    let strict = RepetitionInspector::staged(3, 5).with_semantic(SemanticLoopConfig {
        oscillation_hard_stop: Some(4),
        ..SemanticLoopConfig::default()
    });
    let next = cycle[0].clone();
    let history = completed_calls(cycle, tool_ok("unchanged result"));
    let results = inspect_with_history(&strict, &requests(vec![next]), &history).await;

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(
        results[0].finding_id.as_deref(),
        Some(REPETITION_OSCILLATION_HARD_FINDING_ID)
    );
}

// The whole feature is switchable off, back to exact-signature detection only.
#[tokio::test]
async fn test_semantic_detection_can_be_disabled() {
    let inspector = RepetitionInspector::staged(3, 5).with_semantic(SemanticLoopConfig::disabled());

    let results = inspect(
        &inspector,
        &requests(vec![
            shell("ls /tmp/biorouter-scratch-dir"),
            shell("ls /tmp/biorouter-scratch-di"),
            shell("ls /tmp/biorouter-scratch-d"),
            shell("ls /tmp/biorouter-scratch-"),
        ]),
    )
    .await;

    assert!(
        results.is_empty(),
        "semantic detection was off: {results:?}"
    );
}

// The heuristics look at the current turn. A genuine user message resets the
// window; the agent's own (user-role, user-invisible) loop-guard injections and
// tool responses do not.
#[tokio::test]
async fn test_window_resets_on_a_real_user_turn_only() {
    let inspector = RepetitionInspector::staged(3, 5);

    let earlier: Vec<Message> = [
        "grep -rn 'fn assemble_turn_context' crates/biorouter/src/agents/",
        "grep -rn 'fn assemble_turn_context' crates/biorouter/src/agents",
        "grep -rn 'fn assemble_turn_context' crates/biorouter/src/agent",
    ]
    .iter()
    .enumerate()
    .map(|(index, command)| {
        Message::assistant().with_tool_request(format!("hist_{index}"), Ok(shell(command)))
    })
    .collect();

    let next = requests(vec![shell(
        "grep -rn 'fn assemble_turn_context' crates/biorouter/src/agen",
    )]);

    // Continuing the same turn: the 4th near-duplicate trips the soft stage.
    let results = inspector
        .inspect(
            &next,
            &earlier,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].finding_id.as_deref(),
        Some(REPETITION_NEAR_DUP_SOFT_FINDING_ID)
    );

    // A real user turn in between wipes the slate.
    let mut with_user_turn = earlier.clone();
    with_user_turn.push(Message::user().with_text("actually, try the other crate"));
    let results = inspector
        .inspect(
            &next,
            &with_user_turn,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");
    assert!(
        results.is_empty(),
        "a user turn must reset the loop window: {results:?}"
    );

    // The agent's own loop-guard nudge is user-role but user-invisible — it is
    // not a user turn and must not launder a loop.
    let mut with_guard_nudge = earlier;
    with_guard_nudge.push(
        Message::user()
            .with_text("<biorouter-loop-guard>…</biorouter-loop-guard>")
            .with_visibility(false, true),
    );
    let results = inspector
        .inspect(
            &next,
            &with_guard_nudge,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed");
    assert_eq!(
        results.len(),
        1,
        "an agent-authored injection must not reset the window"
    );
}

// ---------------------------------------------------------------------------
// BR-31: repeated-failing-result / no-progress detection
//
// The gap these cover: the model varies the arguments every time (so neither the
// exact-signature guard nor the near-duplicate heuristic fires) but every call comes
// back with the *same error*. Only the results reveal the loop.
// ---------------------------------------------------------------------------

use biorouter::tool_monitor::{FailureLoopConfig, FAILURE_LOOP_HARD_FINDING_ID};
use rmcp::model::{CallToolResult, Content};

fn tool_error(text: &str) -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(text)],
        structured_content: None,
        is_error: Some(true),
        meta: None,
    }
}

fn tool_ok(text: &str) -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(text)],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    }
}

/// One completed shell call in the transcript: the assistant's request, then the
/// tool response the agent wrote back.
fn completed_shell(index: usize, command: &str, result: CallToolResult) -> Vec<Message> {
    let id = format!("hist_{index}");
    vec![
        Message::assistant().with_tool_request(&id, Ok(shell(command))),
        Message::user().with_tool_response(&id, Ok(result)),
    ]
}

/// Six shell calls, all with different commands (so the call-shape guards stay
/// quiet), every one failing with the same error.
fn six_identical_failures(error: &str) -> Vec<Message> {
    [
        "cargo build",
        "cargo test --workspace",
        "npm run lint",
        "python analyze.py --input data.csv",
        "make release",
        "./scripts/deploy.sh --verbose",
    ]
    .iter()
    .enumerate()
    .flat_map(|(index, command)| completed_shell(index, command, tool_error(error)))
    .collect()
}

async fn inspect_with_history(
    inspector: &RepetitionInspector,
    requests: &[ToolRequest],
    history: &[Message],
) -> Vec<biorouter::tool_inspection::InspectionResult> {
    inspector
        .inspect(
            requests,
            history,
            BioRouterMode::Approve,
            &biorouter::session::Session::default(),
        )
        .await
        .expect("inspection should succeed")
}

#[tokio::test]
async fn test_repeated_identical_failures_stop_the_next_call() {
    let inspector = RepetitionInspector::staged(3, 5);
    let history = six_identical_failures("error: no toolchain installed for 'stable'");
    let next = requests(vec![shell("cargo doc --open")]);

    let results = inspect_with_history(&inspector, &next, &history).await;

    assert_eq!(
        results.len(),
        1,
        "the 7th call must be stopped: {results:?}"
    );
    let deny = &results[0];
    assert_eq!(deny.action, InspectionAction::Deny);
    assert_eq!(
        deny.finding_id.as_deref(),
        Some(FAILURE_LOOP_HARD_FINDING_ID)
    );

    let reason = deny.reason.to_lowercase();
    assert!(
        reason.contains("failed 6 times"),
        "the reason must state the observed streak: {}",
        deny.reason
    );
    assert!(
        reason.contains("did not decline"),
        "the model must not be told the user declined: {}",
        deny.reason
    );
    assert!(
        reason.contains("ask the user") || reason.contains("tell the user"),
        "the reason must offer the honest way out: {}",
        deny.reason
    );
}

// Below the hard threshold the guard stays out of the way — the escalating
// nudges (emitted at the result-collection seam, not here) are what run.
#[tokio::test]
async fn test_a_short_failure_run_is_not_stopped() {
    let inspector = RepetitionInspector::staged(3, 5);
    let mut history = six_identical_failures("error: no toolchain installed for 'stable'");
    history.truncate(10); // five completed calls, not six

    let next = requests(vec![shell("cargo doc --open")]);
    let results = inspect_with_history(&inspector, &next, &history).await;

    assert!(results.is_empty(), "5 failures must not block: {results:?}");
}

// The stop is a circuit breaker, not a lockout: the denial is itself an error
// with a different signature, so it breaks the streak and the model gets to try
// again. (Without this, a tool that failed 6 times would be dead for the rest of
// the turn — and a model that fixed the actual cause could never prove it.)
#[tokio::test]
async fn test_the_stop_clears_itself_so_the_tool_is_not_dead() {
    let inspector = RepetitionInspector::staged(3, 5);
    let mut history = six_identical_failures("error: no toolchain installed for 'stable'");
    let next = requests(vec![shell("cargo doc --open")]);

    let deny = inspect_with_history(&inspector, &next, &history)
        .await
        .into_iter()
        .next()
        .expect("the 7th call is denied");

    // Replay what the agent does with a denial: the reason is written back as the
    // call's (error) result.
    history.push(Message::assistant().with_tool_request("stopped", Ok(shell("cargo doc --open"))));
    history.push(Message::user().with_tool_response("stopped", Ok(tool_error(&deny.reason))));

    let retry = requests(vec![shell("rustup toolchain install stable")]);
    let results = inspect_with_history(&inspector, &retry, &history).await;

    assert!(
        results.is_empty(),
        "after the stop the model must be free to try again: {results:?}"
    );
}

// A tool that succeeds is making progress, whatever it did before.
#[tokio::test]
async fn test_a_success_clears_the_failure_streak() {
    let inspector = RepetitionInspector::staged(3, 5);
    let mut history = six_identical_failures("error: no toolchain installed for 'stable'");
    history.extend(completed_shell(
        99,
        "which cargo",
        tool_ok("/usr/bin/cargo"),
    ));

    let next = requests(vec![shell("cargo doc --open")]);
    let results = inspect_with_history(&inspector, &next, &history).await;

    assert!(results.is_empty(), "a success resets the run: {results:?}");
}

// Another tool's failures are not this tool's loop.
#[tokio::test]
async fn test_the_streak_is_per_tool() {
    let inspector = RepetitionInspector::staged(3, 5);
    let history = six_identical_failures("error: no toolchain installed for 'stable'");

    let next = requests(vec![call_with("read_file", json!({"path": "Cargo.toml"}))]);
    let results = inspect_with_history(&inspector, &next, &history).await;

    assert!(
        results.is_empty(),
        "a different tool must not inherit the streak: {results:?}"
    );
}

// Operators can keep the nudges but never block (`..._HARD_STOP=0`), or turn the
// whole detector off.
#[tokio::test]
async fn test_the_hard_stop_is_configurable_off() {
    let history = six_identical_failures("error: no toolchain installed for 'stable'");
    let next = requests(vec![shell("cargo doc --open")]);

    let nudge_only = RepetitionInspector::staged(3, 5).with_failure_loop(FailureLoopConfig {
        hard_stop_at: None,
        ..FailureLoopConfig::default()
    });
    assert!(inspect_with_history(&nudge_only, &next, &history)
        .await
        .is_empty());

    let off = RepetitionInspector::staged(3, 5).with_failure_loop(FailureLoopConfig::disabled());
    assert!(inspect_with_history(&off, &next, &history).await.is_empty());
}

// A genuine user turn is a fresh start; the agent's own hidden nudge is not.
#[tokio::test]
async fn test_a_user_turn_resets_the_failure_window() {
    let inspector = RepetitionInspector::staged(3, 5);
    let history = six_identical_failures("error: no toolchain installed for 'stable'");
    let next = requests(vec![shell("cargo doc --open")]);

    let mut with_user_turn = history.clone();
    with_user_turn.push(Message::user().with_text("never mind, just read the manifest"));
    assert!(
        inspect_with_history(&inspector, &next, &with_user_turn)
            .await
            .is_empty(),
        "a user turn must reset the failure window"
    );

    let mut with_nudge = history;
    with_nudge.push(
        Message::user()
            .with_text("<biorouter-loop-guard>no progress</biorouter-loop-guard>")
            .with_visibility(false, true),
    );
    assert_eq!(
        inspect_with_history(&inspector, &next, &with_nudge)
            .await
            .len(),
        1,
        "the agent's own nudge must not launder the loop"
    );
}
