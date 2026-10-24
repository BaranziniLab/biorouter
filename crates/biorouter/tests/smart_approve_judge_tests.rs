//! Invocation-scoped regression checks for the optional Smart Approve judge.

use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use biorouter::config::{BioRouterMode, PermissionManager};
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::managed::{ManagedPolicy, ManagedPolicyFile};
use biorouter::model::ModelConfig;
use biorouter::permission::{
    detect_read_only_requests, detect_read_only_tools, PermissionInspector, SmartApproveConfig,
    ToolPermissionStore, ToolRiskRegistry,
};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::Session;
use biorouter::tool_inspection::{InspectionAction, ToolInspector};
use rmcp::model::{CallToolRequestParams, Tool};
use serde_json::{json, Value};

const TOOL: &str = "thirdparty__file_operation";

struct ScriptedJudge {
    verdict: Value,
    fail: bool,
    inputs: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Provider for ScriptedJudge {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "scripted-permission-judge"
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("test-model")
    }

    async fn complete_with_model(
        &self,
        _model: &ModelConfig,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.inputs.lock().unwrap().push(
            messages
                .iter()
                .map(Message::as_concat_text)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if self.fail {
            return Err(ProviderError::ExecutionError("scripted failure".into()));
        }
        Ok((
            Message::assistant().with_tool_request(
                "judge-verdict",
                Ok(CallToolRequestParams {
                    name: "platform__tool_by_tool_permission".into(),
                    arguments: Some(self.verdict.as_object().unwrap().clone()),
                    task: None,
                    meta: None,
                }),
            ),
            ProviderUsage::new("test-model".into(), Usage::default()),
        ))
    }
}

fn provider(verdict: Value, fail: bool) -> (Arc<dyn Provider>, Arc<Mutex<Vec<String>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    (
        Arc::new(ScriptedJudge {
            verdict,
            fail,
            inputs: inputs.clone(),
        }),
        inputs,
    )
}

fn request(id: &str, operation: &str) -> ToolRequest {
    ToolRequest {
        id: id.into(),
        tool_call: Ok(CallToolRequestParams {
            name: TOOL.into(),
            arguments: Some(
                json!({"operation": operation, "path": "SYNTHETIC.txt"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            task: None,
            meta: None,
        }),
        metadata: None,
        tool_meta: None,
    }
}

fn read_verdict() -> Value {
    json!({"read_only_requests": [{"request_id": "read", "tool_name": TOOL}]})
}

#[tokio::test]
async fn judge_receives_arguments_and_distinguishes_same_tool_invocations() {
    let read = request("read", "read");
    let write = request("write", "write");
    let (judge, inputs) = provider(read_verdict(), false);

    let approved = detect_read_only_requests(judge.clone(), vec![&read, &write]).await;
    assert_eq!(approved, vec!["read"]);
    let captured = inputs.lock().unwrap()[0].clone();
    let encoded = captured
        .split("Tool requests: ")
        .nth(1)
        .unwrap()
        .split("\n\n")
        .next()
        .unwrap();
    let supplied: Value = serde_json::from_str(encoded).unwrap();
    assert_eq!(supplied[0]["request_id"], "read");
    assert_eq!(supplied[0]["tool_name"], TOOL);
    assert_eq!(supplied[0]["arguments"]["operation"], "read");
    assert_eq!(supplied[1]["arguments"]["operation"], "write");

    assert!(detect_read_only_tools(judge, vec![&read, &write])
        .await
        .is_empty());
}

#[tokio::test]
async fn malformed_or_unsolicited_judge_decisions_fail_closed() {
    let read = request("read", "read");
    let invalid = [
        json!({"read_only_tools": [TOOL]}),
        json!({"read_only_requests": ["read"]}),
        json!({"read_only_requests": [{"request_id": "read"}]}),
        json!({"read_only_requests": [{"request_id": "other", "tool_name": TOOL}]}),
        json!({"read_only_requests": [{"request_id": "read", "tool_name": "unrequested__writer"}]}),
        json!({"read_only_requests": [
            {"request_id": "read", "tool_name": TOOL},
            {"request_id": "other", "tool_name": TOOL}
        ]}),
    ];
    for verdict in invalid {
        let (judge, _) = provider(verdict.clone(), false);
        assert!(
            detect_read_only_requests(judge, vec![&read])
                .await
                .is_empty(),
            "unsolicited or malformed verdict granted permission: {verdict}"
        );
    }
    let (judge, _) = provider(read_verdict(), true);
    assert!(detect_read_only_requests(judge, vec![&read])
        .await
        .is_empty());
}

#[tokio::test]
async fn duplicate_request_ids_are_not_sent_for_ambiguous_classification() {
    let read = request("read", "read");
    let write = request("read", "write");
    let (judge, inputs) = provider(read_verdict(), false);
    assert!(detect_read_only_requests(judge, vec![&read, &write])
        .await
        .is_empty());
    assert!(inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn inspector_does_not_reuse_judge_approval_for_another_invocation() {
    let fixture = tempfile::tempdir().unwrap();
    let permissions = Arc::new(PermissionManager::new(fixture.path().to_owned()));
    let (judge, inputs) = provider(read_verdict(), false);
    let inspector = PermissionInspector::with_smart_config(
        Arc::new(ToolRiskRegistry::new()),
        permissions.clone(),
        Arc::new(ManagedPolicy::from_file(
            serde_yaml::from_str::<ManagedPolicyFile>("{}").unwrap(),
        )),
        Arc::new(tokio::sync::Mutex::new(Some(judge))),
        SmartApproveConfig {
            judge_unknown: true,
            ..SmartApproveConfig::default()
        },
    )
    .with_scoped_store(Arc::new(RwLock::new(ToolPermissionStore::new())), false);
    let session = Session::default();

    let batch = inspector
        .inspect(
            &[request("read", "read"), request("write", "write")],
            &[],
            BioRouterMode::SmartApprove,
            &session,
        )
        .await
        .unwrap();
    assert_eq!(batch[0].tool_request_id, "read");
    assert_eq!(batch[0].action, InspectionAction::Allow);
    assert_eq!(batch[1].tool_request_id, "write");
    assert!(matches!(
        batch[1].action,
        InspectionAction::RequireApproval(_)
    ));
    assert_eq!(permissions.get_smart_approve_permission(TOOL), None);
    assert!(!permissions.get_config_path().exists());

    let later = inspector
        .inspect(
            &[request("later-write", "write")],
            &[],
            BioRouterMode::SmartApprove,
            &session,
        )
        .await
        .unwrap();
    assert!(matches!(
        later[0].action,
        InspectionAction::RequireApproval(_)
    ));
    assert_eq!(inputs.lock().unwrap().len(), 2);
    assert_eq!(permissions.get_smart_approve_permission(TOOL), None);
}
