//! BR-67: the loop-safety guards must leave a runtime trace.
//!
//! These drive the *production* inspection paths (the repetition guard, the
//! PreToolUse hook veto) and assert that each decision reaches the loop-safety
//! observability sink with enough structure to answer "which guard fired, on
//! which tool, how deep into the run, and did it stop the agent" — and with
//! nothing else: no tool arguments, no reason prose.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::{Arc, Mutex};

use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::hooks::{HookInspector, HooksConfig, HooksManager};
use biorouter::observability::loop_safety::{
    self, LoopSafetyEvent, LoopSafetyKind, LoopSafetyObserver,
};
use biorouter::session::Session;
use biorouter::tool_inspection::ToolInspector;
use biorouter::tool_monitor::RepetitionInspector;
use rmcp::model::CallToolRequestParams;
use rmcp::object;

#[derive(Default)]
struct Spy {
    seen: Mutex<Vec<LoopSafetyEvent>>,
}

impl Spy {
    /// Only this test's session, so the suite's other emitters cannot leak in.
    fn events_for(&self, session_id: &str) -> Vec<LoopSafetyEvent> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect()
    }
}

impl LoopSafetyObserver for Spy {
    fn on_loop_safety_event(&self, event: &LoopSafetyEvent) {
        self.seen.lock().unwrap().push(event.clone());
    }
}

fn spy() -> Arc<Spy> {
    let spy = Arc::new(Spy::default());
    loop_safety::subscribe(spy.clone());
    spy
}

fn session(id: &str) -> Session {
    Session {
        id: id.to_string(),
        ..Session::default()
    }
}

fn tool_request(id: &str, name: &str, args: serde_json::Value) -> ToolRequest {
    let message = Message::assistant().with_tool_request(
        id,
        Ok(CallToolRequestParams {
            task: None,
            meta: None,
            name: name.to_string().into(),
            arguments: args.as_object().cloned(),
        }),
    );
    message
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("message contains a tool request")
        .clone()
}

#[tokio::test]
async fn repetition_stop_is_traced_with_tool_finding_and_run_length() {
    let spy = spy();
    let inspector = RepetitionInspector::new(Some(2));
    let call = || object!({"path": "/etc/hosts"}).into();

    let requests = vec![
        tool_request("c1", "developer__text_editor", call()),
        tool_request("c2", "developer__text_editor", call()),
        // The third identical call is the one the guard denies.
        tool_request("c3", "developer__text_editor", call()),
    ];

    let results = inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &session("obs-repetition-stop"),
        )
        .await
        .expect("inspection succeeds");
    assert_eq!(results.len(), 1, "only the third call is denied");

    let events = spy.events_for("obs-repetition-stop");
    assert_eq!(events.len(), 1, "one verdict, one trace event");
    let event = &events[0];
    assert_eq!(event.kind, LoopSafetyKind::RepetitionStop);
    assert!(event.kind.is_stop(), "a denied call is a stop");
    assert_eq!(event.tool.as_deref(), Some("developer__text_editor"));
    assert_eq!(event.finding_id.as_deref(), Some("REP-001"));
    assert_eq!(
        event.count,
        Some(3),
        "the run length that tripped the guard"
    );

    // Redaction: the arguments of the repeated call never reach the trace.
    let json = serde_json::to_string(event).unwrap();
    assert!(
        !json.contains("/etc/hosts"),
        "trace event must not carry tool arguments: {json}"
    );
}

#[tokio::test]
async fn repetition_soft_warning_is_traced_as_a_non_stop() {
    let spy = spy();
    // Warn on the 2nd identical call, deny on the 4th.
    let inspector = RepetitionInspector::staged(2, 4);
    let call = || object!({"query": "TP53"}).into();

    let requests = vec![
        tool_request("c1", "search", call()),
        tool_request("c2", "search", call()),
    ];

    inspector
        .inspect(
            &requests,
            &[],
            BioRouterMode::Approve,
            &session("obs-repetition-warn"),
        )
        .await
        .expect("inspection succeeds");

    let events = spy.events_for("obs-repetition-warn");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, LoopSafetyKind::RepetitionWarn);
    assert!(!events[0].kind.is_stop(), "the call still ran");
    assert_eq!(events[0].finding_id.as_deref(), Some("REP-002"));
    assert_eq!(events[0].count, Some(2));
}

#[tokio::test]
async fn a_quiet_guard_emits_nothing() {
    let spy = spy();
    let inspector = RepetitionInspector::staged(3, 5);

    inspector
        .inspect(
            &[
                tool_request("c1", "search", object!({"query": "TP53"}).into()),
                tool_request("c2", "search", object!({"query": "BRCA1"}).into()),
            ],
            &[],
            BioRouterMode::Approve,
            &session("obs-quiet"),
        )
        .await
        .expect("inspection succeeds");

    assert!(
        spy.events_for("obs-quiet").is_empty(),
        "no guard fired, so nothing is traced"
    );
}

#[tokio::test]
async fn hook_block_and_hook_ask_are_traced() {
    let spy = spy();
    let config: HooksConfig = serde_yaml::from_str(
        r#"
PreToolUse:
  - matcher: "developer__shell"
    hooks:
      - type: command
        command: "echo 'rm is forbidden here' >&2; exit 2"
"#,
    )
    .expect("test yaml parses");
    let inspector = HookInspector::new(Arc::new(HooksManager::with_config(
        config,
        true,
        Arc::new(tokio::sync::Mutex::new(None)),
    )));

    let results = inspector
        .inspect(
            &[tool_request(
                "req_1",
                "developer__shell",
                object!({"command": "rm -rf /tmp/secret-payload"}).into(),
            )],
            &[],
            BioRouterMode::Approve,
            &session("obs-hook-block"),
        )
        .await
        .expect("inspection succeeds");
    assert_eq!(results.len(), 1, "the hook denied the call");

    let events = spy.events_for("obs-hook-block");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, LoopSafetyKind::HookBlock);
    assert!(events[0].kind.is_stop());
    assert_eq!(events[0].tool.as_deref(), Some("developer__shell"));

    // Neither the blocked command nor the hook's own reason is traced.
    let json = serde_json::to_string(&events[0]).unwrap();
    assert!(!json.contains("secret-payload"), "{json}");
    assert!(!json.contains("forbidden"), "{json}");

    // An `ask` escalation is traced too, but as a non-stop: the user decides.
    let ask_config: HooksConfig = serde_yaml::from_str(
        r#"
PreToolUse:
  - hooks:
      - type: command
        command: "echo '{\"hookSpecificOutput\":{\"permissionDecision\":\"ask\",\"permissionDecisionReason\":\"hook wants eyes on this\"}}'"
"#,
    )
    .expect("test yaml parses");
    let ask_inspector = HookInspector::new(Arc::new(HooksManager::with_config(
        ask_config,
        true,
        Arc::new(tokio::sync::Mutex::new(None)),
    )));
    ask_inspector
        .inspect(
            &[tool_request("req_1", "anything", object!({}).into())],
            &[],
            BioRouterMode::Approve,
            &session("obs-hook-ask"),
        )
        .await
        .expect("inspection succeeds");

    let ask_events = spy.events_for("obs-hook-ask");
    assert_eq!(ask_events.len(), 1);
    assert_eq!(ask_events[0].kind, LoopSafetyKind::HookAsk);
    assert!(!ask_events[0].kind.is_stop(), "an ask is not a stop");
}

#[tokio::test]
async fn counters_accumulate_across_guards() {
    let spy = spy();
    let inspector = RepetitionInspector::new(Some(1));
    let call = || object!({"path": "/tmp/x"}).into();

    inspector
        .inspect(
            &[
                tool_request("c1", "tool_a", call()),
                tool_request("c2", "tool_a", call()),
                tool_request("c3", "tool_a", call()),
            ],
            &[],
            BioRouterMode::Approve,
            &session("obs-counters"),
        )
        .await
        .expect("inspection succeeds");

    assert_eq!(spy.events_for("obs-counters").len(), 2, "two calls denied");
    let counters = loop_safety::counters();
    assert!(
        counters.get("repetition_stop").copied().unwrap_or(0) >= 2,
        "the per-kind counter answers 'is this guard actually firing': {counters:?}"
    );
}
