//! **The first-crossing disclosure, end to end** (issue #56, design §7's `✓!`
//! cells): a private-capability conversation writing into a PUBLIC one shows the
//! user the exact payload before it is sent, once per (caller, target) pair.
//!
//! # Why this is an integration binary and not a unit test
//!
//! Two of the three inputs are process-global. `WorkspaceCrossingInspector`
//! resolves the target through `SessionManager::instance()` — the real store,
//! which a unit test in `workspace_extension`'s own module deliberately does not
//! use — and `privacy::crossing`'s ledger is a process-global set. So the test
//! needs a whole process it can point at a temp directory, which is what a
//! separate test binary is.
//!
//! ⚠ **`BIOROUTER_PATH_ROOT` must be set before ANYTHING touches the store.**
//! `SESSION_STORAGE` is a `LazyLock` over `Paths::data_dir()`; once it has been
//! forced, this test would be reading and writing the developer's own
//! `sessions.db`. It is set once, by the first test to run, and every test in
//! this file goes through `fixture()` so none of them can forget.

use std::sync::Arc;

use async_trait::async_trait;
use biorouter::agents::workspace_inspector::WorkspaceCrossingInspector;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::{SessionManager, SessionType};
use biorouter::tool_inspection::{InspectionAction, ToolInspector};
use rmcp::model::Tool;

/// A provider that exists only to answer `tier()`. Named after a real private
/// provider so nothing downstream can decide it is public by name.
struct InstitutionalModel;

#[async_trait]
impl Provider for InstitutionalModel {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }
    fn get_name(&self) -> &str {
        "versa_azure"
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("test-model")
    }
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        unreachable!("this fixture exists only to be asked its tier")
    }
}

fn send_prompt_request(id: &str, target: &str, text: &str) -> ToolRequest {
    ToolRequest {
        id: id.to_string(),
        tool_call: Ok(rmcp::model::CallToolRequestParams {
            name: "workspace_send_prompt".into(),
            arguments: Some(
                serde_json::json!({ "session_id": target, "mode": "note", "text": text })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        }),
        metadata: Default::default(),
        tool_meta: Default::default(),
    }
}

/// Point the process-global store at a temp directory, once, before anything
/// forces the `LazyLock`. Serialized because `set_var` is, and because the
/// crossing ledger below it is process-global too.
fn store_root() -> &'static tempfile::TempDir {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::TempDir::new().unwrap();
        // SAFETY: inside a `OnceLock` initializer, so exactly one thread runs
        // it, and it runs before any test in this binary reads the store.
        unsafe {
            std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
        }
        root
    })
}

#[tokio::test]
#[serial_test::serial(workspace_crossing)]
async fn a_private_chat_writing_into_a_public_one_discloses_its_payload_exactly_once() {
    let root = store_root();

    const PAYLOAD: &str = "the MS cohort's 2019 relapse counts, verbatim";

    let sm = SessionManager::instance();
    let caller = sm
        .create_session(
            root.path().to_path_buf(),
            "private lead".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let public_target = sm
        .create_session(
            root.path().to_path_buf(),
            "public worker".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let private_target = sm
        .create_session(
            root.path().to_path_buf(),
            "private peer".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    sm.update(&private_target.id)
        .raise_privacy(
            SessionClassification::Private,
            "test:workspace-crossing-disclosure",
        )
        .apply()
        .await
        .unwrap();
    // The ratchet really fired. Without this the "same tier, no disclosure"
    // assertion below would pass against a target that is merely public.
    assert_eq!(
        sm.get_session(&private_target.id, false)
            .await
            .unwrap()
            .privacy_tier,
        SessionClassification::Private,
    );

    let provider: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(InstitutionalModel))));
    let inspector = WorkspaceCrossingInspector::new(provider);

    let caller_session = sm.get_session(&caller.id, false).await.unwrap();
    let inspect = |request: ToolRequest| {
        let inspector = &inspector;
        let session = caller_session.clone();
        async move {
            inspector
                .inspect(&[request], &[], BioRouterMode::Auto, &session)
                .await
                .unwrap()
        }
    };

    // 1. The crossing. Private caller, public target, first time.
    let results = inspect(send_prompt_request("req-1", &public_target.id, PAYLOAD)).await;
    assert_eq!(results.len(), 1, "no disclosure was raised: {results:?}");
    let prompt = match &results[0].action {
        InspectionAction::RequireApproval(Some(prompt)) => prompt.clone(),
        other => panic!("the disclosure must be a RequireApproval carrying text, got {other:?}"),
    };
    // The payload, VERBATIM. A card that summarised what was about to be sent
    // would leave the user approving something they cannot check — and the whole
    // point of this gate is that they can.
    assert!(
        prompt.contains(PAYLOAD),
        "the approval did not show the payload: {prompt}"
    );
    assert!(
        prompt.contains(&public_target.id),
        "the approval did not name the target: {prompt}"
    );

    // 2. Asking again — with no write having landed — asks again. This is the
    //    denial case: a user who says no must be asked on the retry, not
    //    silently obeyed.
    let again = inspect(send_prompt_request("req-2", &public_target.id, PAYLOAD)).await;
    assert_eq!(
        again.len(),
        1,
        "a second attempt was let through, so denying the first would have bought \
         silence for it"
    );

    // 3. Once the write has landed, the pair has crossed and stops asking.
    biorouter::privacy::crossing::record(&caller.id, &public_target.id);
    let after = inspect(send_prompt_request("req-3", &public_target.id, PAYLOAD)).await;
    assert!(
        after.is_empty(),
        "the disclosure repeated for a pair that has already crossed: {after:?}"
    );

    // 4. A DIFFERENT public target is a different crossing. Keying the ledger on
    //    the caller alone would let one approval cover every public conversation
    //    on the machine.
    let other_public = sm
        .create_session(
            root.path().to_path_buf(),
            "another worker".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let second = inspect(send_prompt_request("req-4", &other_public.id, PAYLOAD)).await;
    assert_eq!(second.len(), 1, "a second public target was not disclosed");

    // 5. A same-tier write crosses nothing, so it discloses nothing. This is the
    //    control that keeps the assertions above from being "the inspector fires
    //    for everything".
    let same_tier = inspect(send_prompt_request("req-5", &private_target.id, PAYLOAD)).await;
    assert!(
        same_tier.is_empty(),
        "a private→private write raised a crossing disclosure: {same_tier:?}"
    );
}

/// **A first crossing cannot be made from inside an `execute_code` script.**
///
/// The disclosure is a `ToolInspector`, and the JS sandbox hands a script's
/// inner tool calls straight to `ExtensionManager::dispatch_tool_call` — a door
/// no inspector sees. That is why `code_execution_extension` already carries
/// boundary refusals for the global memory store and the session database; this
/// is the third.
///
/// The stakes are higher than one skipped card: the handler records the pair as
/// crossed afterwards, so a single un-inspected script call would silence the
/// disclosure for that pair permanently.
#[tokio::test]
#[serial_test::serial(workspace_crossing)]
async fn a_first_crossing_is_refused_at_the_uninspected_script_boundary() {
    use biorouter::agents::workspace_inspector::uninspected_crossing_refusal;
    use biorouter::privacy::CallCapability;
    use biorouter::security::UninspectedBoundary;

    let root = store_root();
    let sm = SessionManager::instance();
    let public_target = sm
        .create_session(
            root.path().to_path_buf(),
            "script target".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let private_target = sm
        .create_session(
            root.path().to_path_buf(),
            "script peer".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    sm.update(&private_target.id)
        .raise_privacy(SessionClassification::Private, "test:script-boundary")
        .apply()
        .await
        .unwrap();

    let obj = |v: serde_json::Value| v.as_object().unwrap().clone();
    let send = |target: &str| {
        obj(serde_json::json!({
            "session_id": target, "mode": "turn", "text": "the cohort, verbatim"
        }))
    };
    // Built through the PUBLIC constructor a production caller uses. There is a
    // test-only one, but it is `#[cfg(test)]` and so invisible from an
    // integration binary — which is the point of the census in
    // `tests/privacy_capability.rs`: a capability comes from `sample` or from
    // `public_enforced`, and nowhere else.
    let institutional: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(InstitutionalModel))));
    let private_caller = CallCapability::sample(&institutional).await;
    assert!(
        private_caller.tier().is_private() && private_caller.enforced(),
        "the fixture caller is not a private, enforced capability, so nothing below \
         is testing the disclosure"
    );

    let refusal = uninspected_crossing_refusal(
        private_caller,
        "script-caller",
        "workspace_send_prompt",
        Some(&send(&public_target.id)),
        UninspectedBoundary::ExecuteCodeScript,
    )
    .await
    .expect("a first crossing from a script must be refused, not silently recorded");
    assert!(refusal.contains(&public_target.id), "{refusal}");
    // The refusal has to say what to do instead, or the model retries the same
    // call until it gives up.
    assert!(refusal.contains("directly"), "{refusal}");

    // Narrow by construction — each of these must pass straight through.
    // 1. Same tier: nothing is crossing.
    assert!(uninspected_crossing_refusal(
        private_caller,
        "script-caller",
        "workspace_send_prompt",
        Some(&send(&private_target.id)),
        UninspectedBoundary::ExecuteCodeScript,
    )
    .await
    .is_none());
    // 2. A public caller: it cannot reach a private target at all, and a
    //    public→public write crosses nothing.
    assert!(uninspected_crossing_refusal(
        CallCapability::public_enforced(),
        "script-caller",
        "workspace_send_prompt",
        Some(&send(&public_target.id)),
        UninspectedBoundary::ExecuteCodeScript,
    )
    .await
    .is_none());
    // 3. A tool with no payload.
    assert!(uninspected_crossing_refusal(
        private_caller,
        "script-caller",
        "workspace_close",
        Some(&obj(serde_json::json!({
            "session_id": public_target.id, "scope": "turn"
        }))),
        UninspectedBoundary::ExecuteCodeScript,
    )
    .await
    .is_none());
    // 4. A pair the user has already approved. This is the one that keeps the
    //    refusal from becoming "scripts may never touch the workspace".
    biorouter::privacy::crossing::record("script-caller", &public_target.id);
    assert!(
        uninspected_crossing_refusal(
            private_caller,
            "script-caller",
            "workspace_send_prompt",
            Some(&send(&public_target.id)),
            UninspectedBoundary::ExecuteCodeScript,
        )
        .await
        .is_none(),
        "a pair the user already approved must not be refused for ever after"
    );
}

/// **A child born under this chat's public ceiling asks on every payload**
/// (DR-33, design §3.4). A chat that was public by a decision model spawned
/// it; the chat has since become private. Every write into that child is a
/// private-to-public crossing, and one approval must not let a now-private chat
/// stream its content into a child it started while it was public.
#[tokio::test]
#[serial_test::serial(workspace_crossing)]
async fn a_child_under_the_callers_public_ceiling_asks_on_every_payload() {
    use biorouter::agents::workspace_inspector::uninspected_crossing_refusal;
    use biorouter::privacy::decision_axes::PrivacyCeiling;
    use biorouter::privacy::CallCapability;
    use biorouter::security::UninspectedBoundary;

    let root = store_root();
    let sm = SessionManager::instance();
    let caller = sm
        .create_session(
            root.path().to_path_buf(),
            "once public lead".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let mut child = sm
        .create_session(
            root.path().to_path_buf(),
            "ceilinged child".into(),
            SessionType::SubAgent,
        )
        .await
        .unwrap();
    PrivacyCeiling::public_from(caller.id.as_str(), None)
        .stamp(&mut child.extension_data)
        .unwrap();
    sm.update(&child.id)
        .parent_session_id(Some(caller.id.clone()))
        .extension_data(child.extension_data.clone())
        .apply()
        .await
        .unwrap();

    let provider: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(InstitutionalModel))));
    let inspector = WorkspaceCrossingInspector::new(provider.clone());
    let caller_session = sm.get_session(&caller.id, false).await.unwrap();
    let inspect = |id: &'static str| {
        let inspector = &inspector;
        let session = caller_session.clone();
        let target = child.id.clone();
        async move {
            inspector
                .inspect(
                    &[send_prompt_request(id, &target, "the cohort")],
                    &[],
                    BioRouterMode::Auto,
                    &session,
                )
                .await
                .unwrap()
        }
    };

    let first = inspect("ceiling-1").await;
    let prompt = match first.as_slice() {
        [result] => match &result.action {
            InspectionAction::RequireApproval(Some(prompt)) => prompt.clone(),
            other => panic!("expected a disclosure, got {other:?}"),
        },
        other => panic!("expected one disclosure, got {other:?}"),
    };
    assert!(prompt.contains("asks again for every message"), "{prompt}");
    assert!(!prompt.contains("stops asking"), "{prompt}");

    // The write lands and the pair is recorded, and it asks again anyway.
    biorouter::privacy::crossing::record(&caller.id, &child.id);
    assert_eq!(
        inspect("ceiling-2").await.len(),
        1,
        "a remembered approval silenced a write into a child under the caller's ceiling"
    );

    // The script boundary refuses it every time too.
    let private_caller = CallCapability::sample(&provider).await;
    let args = serde_json::json!({ "session_id": child.id, "mode": "turn", "text": "x" });
    assert!(uninspected_crossing_refusal(
        private_caller,
        &caller.id,
        "workspace_send_prompt",
        Some(args.as_object().unwrap()),
        UninspectedBoundary::ExecuteCodeScript,
    )
    .await
    .is_some());

    // Another chat's child is the ordinary once-per-pair rule.
    let stranger = sm
        .create_session(
            root.path().to_path_buf(),
            "stranger".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    biorouter::privacy::crossing::record(&stranger.id, &child.id);
    let stranger_session = sm.get_session(&stranger.id, false).await.unwrap();
    let results = inspector
        .inspect(
            &[send_prompt_request("ceiling-3", &child.id, "x")],
            &[],
            BioRouterMode::Auto,
            &stranger_session,
        )
        .await
        .unwrap();
    assert!(results.is_empty(), "{results:?}");
}
