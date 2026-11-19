use super::*;
use crate::agents::mcp_client::{McpClientTrait, McpMeta};
use crate::config::permission::PermissionManager;
use crate::conversation::message::ActionRequiredData;
#[cfg(unix)]
use crate::hooks::{HookDefinition, HookEvent, HookMatcherGroup};
use crate::hooks::{HooksConfig, HooksManager};
use crate::managed::{ManagedPolicy, ManagedPolicyFile};
use crate::model::ModelConfig;
use crate::providers::base::{ProviderMetadata, ProviderUsage, Usage};
use crate::providers::errors::ProviderError;
use crate::session::session_manager::SessionType;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tempfile::TempDir;

const CANONICAL: &str = "code_execution__execute_code";
const HIDDEN: &str = "ucsfomopagent__execute_code";

type Calls = Arc<StdMutex<Vec<(String, Option<rmcp::model::JsonObject>)>>>;

struct ProbeClient {
    name: &'static str,
    calls: Calls,
}

#[async_trait]
impl McpClientTrait for ProbeClient {
    async fn list_tools(
        &self,
        _cursor: Option<String>,
        _cancel: CancellationToken,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ServiceError> {
        let schema = json!({
            "type": "object",
            "properties": {"count": {"type": "integer"}},
            "required": ["count"]
        });
        Ok(rmcp::model::ListToolsResult {
            tools: vec![Tool::new(
                self.name,
                "Synthetic counted tool: no code or command is executed",
                schema.as_object().unwrap().clone(),
            )],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<rmcp::model::JsonObject>,
        _meta: McpMeta,
        _cancel: CancellationToken,
    ) -> Result<CallToolResult, rmcp::ServiceError> {
        self.calls
            .lock()
            .unwrap()
            .push((name.to_string(), arguments));
        Ok(CallToolResult::success(vec![Content::text(
            "ISSUE545_PROBE_RAN",
        )]))
    }

    fn get_info(&self) -> Option<&rmcp::model::InitializeResult> {
        None
    }
}

struct AliasProvider {
    name: &'static str,
    calls: AtomicUsize,
    offered: StdMutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Provider for AliasProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new("issue545-mock", "Issue 545", "", "mock", vec![], "", vec![])
    }

    fn get_name(&self) -> &str {
        "issue545-mock"
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("mock").with_context_limit(Some(1_000_000))
    }

    async fn complete_with_model(
        &self,
        _model: &ModelConfig,
        system: &str,
        _messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new("mock".to_string(), Usage::default());
        if system.contains("You are a loop monitor") {
            return Ok((
                Message::assistant().with_text(r#"{"looping":false}"#),
                usage,
            ));
        }
        self.offered
            .lock()
            .unwrap()
            .push(tools.iter().map(|t| t.name.to_string()).collect());
        let message = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Message::assistant().with_tool_request(
                "alias-545",
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: self.name.into(),
                    arguments: Some(rmcp::object!({"count": "7"})),
                }),
            )
        } else {
            Message::assistant().with_text("ISSUE545_FINISHED")
        };
        Ok((message, usage))
    }
}

struct Fixture {
    agent: Agent,
    session: Session,
    provider: Arc<AliasProvider>,
    public_calls: Calls,
    hidden_calls: Calls,
    _root: TempDir,
}

#[cfg(unix)]
fn command_hook(event: HookEvent, matcher: &str, command: &str) -> HooksConfig {
    let mut config = HooksConfig::default();
    config.events.insert(
        event,
        vec![HookMatcherGroup {
            matcher: Some(matcher.to_string()),
            input_matcher: None,
            hooks: vec![HookDefinition::Command {
                command: command.to_string(),
                timeout: Some(5),
            }],
        }],
    );
    config
}

impl Fixture {
    async fn new(name: &'static str, managed_yaml: &str, hooks_config: HooksConfig) -> Self {
        let root = TempDir::new().unwrap();
        let sessions = Arc::new(SessionManager::new(root.path().join("sessions")));
        let permissions = Arc::new(PermissionManager::new(root.path().join("permissions")));
        let mut agent = Agent::with_config(
            AgentConfig::new(
                sessions.clone(),
                permissions.clone(),
                None,
                BioRouterMode::Auto,
            )
            .with_project_hooks(false),
        );
        let managed = Arc::new(ManagedPolicy::from_file(
            serde_yaml::from_str::<ManagedPolicyFile>(managed_yaml).unwrap(),
        ));
        let hooks = Arc::new(HooksManager::with_config_and_managed(
            hooks_config,
            false,
            agent.provider.clone(),
            managed.clone(),
        ));
        agent.tool_inspection_manager = Arc::new(Agent::create_tool_inspection_manager(
            permissions,
            hooks.clone(),
            managed,
            agent.tool_risks.clone(),
            agent.provider.clone(),
            sessions.clone(),
        ));
        agent.hooks_manager = hooks;
        let public_calls = Arc::new(StdMutex::new(Vec::new()));
        let hidden_calls = Arc::new(StdMutex::new(Vec::new()));
        let config = ExtensionConfig::stdio(
            "code_execution",
            "unused-issue545-fixture",
            "fixture",
            30_u64,
        );
        agent
            .extension_manager
            .add_client(
                config.key(),
                config,
                Arc::new(ProbeClient {
                    name: "execute_code",
                    calls: public_calls.clone(),
                }),
                None,
                None,
            )
            .await;
        let config = ExtensionConfig::stdio(
            "ucsfomopagent",
            "unused-issue545-private",
            "fixture",
            30_u64,
        );
        agent
            .extension_manager
            .add_client(
                config.key(),
                config,
                Arc::new(ProbeClient {
                    name: "execute_code",
                    calls: hidden_calls.clone(),
                }),
                None,
                None,
            )
            .await;
        let session = sessions
            .create_session(
                root.path().to_path_buf(),
                "issue545".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        let provider = Arc::new(AliasProvider {
            name,
            calls: AtomicUsize::new(0),
            offered: StdMutex::new(Vec::new()),
        });
        agent
            .update_provider(provider.clone(), &session.id)
            .await
            .unwrap();
        Self {
            agent,
            session,
            provider,
            public_calls,
            hidden_calls,
            _root: root,
        }
    }

    async fn run(&self) -> (Vec<String>, Value) {
        let drain = async {
            let mut cards = Vec::new();
            let mut stream = self
                .agent
                .reply(
                    Message::user().with_text("Use the synthetic offered probe once."),
                    SessionConfig {
                        id: self.session.id.clone(),
                        schedule_id: None,
                        max_turns: Some(3),
                        max_tool_calls: Some(3),
                        budget: None,
                        retry_config: None,
                        reasoning_effort: None,
                    },
                    None,
                )
                .await
                .unwrap();
            while let Some(event) = stream.next().await {
                if let AgentEvent::Message(message) = event.unwrap() {
                    for content in message.content {
                        if let MessageContent::ActionRequired(action) = content {
                            if let ActionRequiredData::ToolConfirmation { id, tool_name, .. } =
                                action.data
                            {
                                cards.push(tool_name);
                                self.agent.handle_confirmation(id, crate::permission::PermissionConfirmation {
                                    principal_type: crate::permission::permission_confirmation::PrincipalType::Tool,
                                    permission: crate::permission::Permission::DenyOnce,
                                }).await;
                            }
                        }
                    }
                }
            }
            cards
        };
        let cards = tokio::time::timeout(Duration::from_secs(15), drain)
            .await
            .expect("synthetic loop or controlled denial did not finish");
        let stored = self
            .agent
            .config
            .session_manager
            .get_session(&self.session.id, true)
            .await
            .unwrap();
        let transcript = serde_json::to_value(stored.conversation.unwrap().messages()).unwrap();
        let has_code_execution = self
            .agent
            .extension_manager
            .is_extension_enabled("code_execution")
            .await;
        let offered = self.provider.offered.lock().unwrap();
        assert!(
            !offered.is_empty(),
            "fixture did not issue a provider request"
        );
        if has_code_execution {
            assert!(
                offered
                    .iter()
                    .all(|tools| tools.iter().any(|name| name == CANONICAL)),
                "fixture never offered the canonical public alias target"
            );
        }
        assert!(
            offered
                .iter()
                .all(|tools| !tools.iter().any(|name| name == HIDDEN)),
            "Gate E disclosed the private fixture tool"
        );
        assert!(
            self.hidden_calls.lock().unwrap().is_empty(),
            "private fixture dispatched"
        );
        (cards, transcript)
    }
}

#[tokio::test]
async fn issue545_canonical_policy_and_private_roster_controls() {
    let fixture = Fixture::new(
        CANONICAL,
        "permissions:\n  deny: [\"code_execution__execute_code\"]\n",
        HooksConfig::default(),
    )
    .await;
    let (cards, transcript) = fixture.run().await;
    assert!(cards.is_empty());
    assert!(fixture.public_calls.lock().unwrap().is_empty());
    assert!(transcript
        .to_string()
        .contains(crate::permission::managed_inspector::MANAGED_DENY_REASON));

    let fixture = Fixture::new(
        CANONICAL,
        "permissions:\n  ask: [\"code_execution__execute_code\"]\n",
        HooksConfig::default(),
    )
    .await;
    let (cards, _) = fixture.run().await;
    assert_eq!(cards, vec![CANONICAL.to_string()]);
    assert!(fixture.public_calls.lock().unwrap().is_empty());

    #[cfg(unix)]
    {
        let hooks = command_hook(
            HookEvent::PreToolUse,
            CANONICAL,
            "printf '%s' 'ISSUE545_CANONICAL_CONTROL' >&2; exit 2",
        );
        let fixture = Fixture::new(CANONICAL, "{}", hooks).await;
        let (cards, transcript) = fixture.run().await;
        assert!(cards.is_empty());
        assert!(fixture.public_calls.lock().unwrap().is_empty());
        assert!(transcript
            .to_string()
            .contains("ISSUE545_CANONICAL_CONTROL"));
    }

    let fixture = Fixture::new("absent__execute_code", "{}", HooksConfig::default()).await;
    let (cards, transcript) = fixture.run().await;
    assert!(cards.is_empty());
    assert!(fixture.public_calls.lock().unwrap().is_empty());
    assert!(!transcript.to_string().contains(HIDDEN));
}

#[tokio::test]
async fn issue545_canonical_managed_deny_also_blocks_the_bare_alias_before_dispatch() {
    for pattern in [CANONICAL, "code_execution__.*"] {
        for name in [CANONICAL, "execute_code"] {
            let fixture = Fixture::new(
                name,
                &format!("permissions:\n  deny: [\"{pattern}\"]\n"),
                HooksConfig::default(),
            )
            .await;
            let (cards, transcript) = fixture.run().await;
            assert!(
                fixture.public_calls.lock().unwrap().is_empty(),
                "managed canonical deny was bypassed by {name}"
            );
            assert!(cards.is_empty());
            assert!(transcript
                .to_string()
                .contains(crate::permission::managed_inspector::MANAGED_DENY_REASON));
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn issue545_canonical_managed_ask_survives_alias_and_permission_hook_allow() {
    for name in [CANONICAL, "execute_code"] {
        let hooks = command_hook(
            HookEvent::PermissionRequest,
            CANONICAL,
            "printf '%s' '{\"hookSpecificOutput\":{\"permissionDecision\":\"allow\"}}'",
        );
        let fixture = Fixture::new(
            name,
            "permissions:\n  ask: [\"code_execution__execute_code\"]\n",
            hooks,
        )
        .await;
        let (cards, _) = fixture.run().await;
        assert_eq!(
            cards,
            vec![CANONICAL.to_string()],
            "managed ask lost canonical identity"
        );
        assert!(
            fixture.public_calls.lock().unwrap().is_empty(),
            "an automated hook must not grant a human-only managed ask"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn issue545_canonical_pre_tool_use_deny_blocks_the_bare_alias() {
    for name in [CANONICAL, "execute_code"] {
        let hooks = command_hook(
            HookEvent::PreToolUse,
            CANONICAL,
            "printf '%s' 'ISSUE545_HOOK_DENY' >&2; exit 2",
        );
        let fixture = Fixture::new(name, "{}", hooks).await;
        let (cards, transcript) = fixture.run().await;
        assert!(
            fixture.public_calls.lock().unwrap().is_empty(),
            "canonical hook missed {name}"
        );
        assert!(cards.is_empty());
        assert!(transcript.to_string().contains("ISSUE545_HOOK_DENY"));
    }
}

fn find_audit(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(object) => object
            .get("biorouterToolExecution")
            .or_else(|| object.values().find_map(find_audit)),
        Value::Array(items) => items.iter().find_map(find_audit),
        _ => None,
    }
}

#[tokio::test]
async fn issue545_unique_offered_alias_is_coerced_and_preserves_execution_audit() {
    let fixture = Fixture::new("execute_code", "{}", HooksConfig::default()).await;
    let (cards, transcript) = fixture.run().await;
    assert!(cards.is_empty());
    let calls = fixture.public_calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        1,
        "offered alias did not dispatch exactly once"
    );
    assert_eq!(calls[0].0, "execute_code");
    assert_eq!(
        calls[0].1.as_ref().unwrap()["count"],
        json!(7),
        "canonical schema coercion was skipped"
    );
    let audit = find_audit(&transcript).expect("alias rewrite lost raw/executed audit");
    assert_eq!(audit["providerAuthored"]["name"], "execute_code");
    assert_eq!(audit["actuallyExecuted"]["name"], CANONICAL);
}

#[tokio::test]
async fn issue545_ambiguous_or_prefixed_unknown_names_never_dispatch_or_disclose_private_hints() {
    for name in ["probe", "absent__execute_code"] {
        let fixture = Fixture::new(name, "{}", HooksConfig::default()).await;
        fixture
            .agent
            .extension_manager
            .remove_extension("code_execution")
            .await
            .unwrap();
        for owner in ["left545", "right545"] {
            let config =
                ExtensionConfig::stdio(owner, "unused-issue545-ambiguous", "fixture", 30_u64);
            fixture
                .agent
                .extension_manager
                .add_client(
                    config.key(),
                    config,
                    Arc::new(ProbeClient {
                        name: "probe",
                        calls: fixture.public_calls.clone(),
                    }),
                    None,
                    None,
                )
                .await;
        }
        let (cards, transcript) = fixture.run().await;
        let text = transcript.to_string();
        assert!(
            fixture.public_calls.lock().unwrap().is_empty(),
            "ambiguous or prefixed unknown call ran"
        );
        assert!(cards.is_empty());
        assert!(
            !text.contains(HIDDEN),
            "hint disclosed a withheld private tool"
        );
        if name == "probe" {
            assert!(
                fixture
                    .provider
                    .offered
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|tools| tools.iter().any(|name| name == "left545__probe")
                        && tools.iter().any(|name| name == "right545__probe")),
                "ambiguous candidates were not actually offered to this turn"
            );
            assert!(
                text.contains("left545__probe") && text.contains("right545__probe"),
                "not-found hint must identify only the offered ambiguous candidates"
            );
        }
    }
}
