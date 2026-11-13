use biorouter::agents::subagent_tool::{create_subagent_tool, SUBAGENT_TOOL_NAME};
use biorouter::workflow::{SubWorkflow, Workflow};
use std::collections::HashMap;
use tempfile::TempDir;

/// Every test here runs against a throwaway config root, so a spawn that
/// reads config never reaches the person's own.
#[ctor::ctor]
fn sandbox() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = TempDir::new().expect("a sandbox");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

const WORKFLOW_TWO_PARAMS: &str = r#"
version: "1.0.0"
title: "Test Task"
description: "A test task"
instructions: "Process {{ first }} and {{ second }}"
parameters:
  - key: first
    input_type: string
    requirement: required
    description: "First param"
  - key: second
    input_type: string
    requirement: required
    description: "Second param"
"#;

fn write_workflow(temp_dir: &TempDir, name: &str, content: &str) -> String {
    let path = temp_dir.path().join(format!("{}.yaml", name));
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().to_string()
}

fn make_subworkflow(
    path: String,
    name: &str,
    values: Option<HashMap<String, String>>,
) -> SubWorkflow {
    SubWorkflow {
        name: name.to_string(),
        path,
        values,
        sequential_when_repeated: false,
        description: Some(format!("{} description", name)),
    }
}

#[test]
fn test_tool_description_includes_subworkflow_params_and_filters_presets() {
    let temp_dir = TempDir::new().unwrap();
    let path = write_workflow(&temp_dir, "mytask", WORKFLOW_TWO_PARAMS);

    let no_presets = make_subworkflow(path.clone(), "mytask", None);
    let tool = create_subagent_tool(&[no_presets]);
    let desc = tool.description.as_ref().unwrap();
    assert!(desc.contains("mytask"));
    assert!(desc.contains("first [required]"));
    assert!(desc.contains("second [required]"));

    let mut preset = HashMap::new();
    preset.insert("second".to_string(), "preset_value".to_string());
    let with_presets = make_subworkflow(path, "deploy", Some(preset));
    let tool = create_subagent_tool(&[with_presets]);
    let params_section = tool
        .description
        .as_ref()
        .unwrap()
        .split("(params:")
        .nth(1)
        .unwrap_or("");
    assert!(params_section.contains("first"));
    assert!(!params_section.contains("second"));
}

#[test]
fn test_adhoc_workflow_builder_and_security_check() {
    let workflow = Workflow::builder()
        .version("1.0.0")
        .title("Adhoc Task")
        .description("An ad-hoc task")
        .instructions("Do the thing")
        .build()
        .unwrap();

    assert_eq!(workflow.title, "Adhoc Task");
    assert_eq!(workflow.instructions.as_ref().unwrap(), "Do the thing");
    assert!(!workflow.check_for_security_warnings());
}

#[test]
fn test_adhoc_tool_schema_properties() {
    let tool = create_subagent_tool(&[]);

    assert_eq!(tool.name, SUBAGENT_TOOL_NAME);
    assert!(tool.description.as_ref().unwrap().contains("Ad-hoc"));
    assert!(!tool
        .description
        .as_ref()
        .unwrap()
        .contains("Available subworkflows"));

    let props = tool
        .input_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(props.contains_key("instructions"));
    assert!(props.contains_key("subworkflow"));
    assert!(props.contains_key("parameters"));
    assert!(props.contains_key("extensions"));
    assert!(props.contains_key("settings"));
    assert!(props.contains_key("summary"));
}

/// BR-71 §4.5: the merged spawn tool carries the two visibility parameters
/// Task 36 resolves. Asserted from an integration test (not just the unit one)
/// because this is the schema an out-of-crate consumer sees.
#[test]
fn adhoc_tool_schema_carries_the_br71_visibility_params() {
    let tool = create_subagent_tool(&[]);

    let props = tool
        .input_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(props.contains_key("visible"));
    assert!(props.contains_key("placement"));
    assert_eq!(
        props["placement"]["enum"],
        serde_json::json!(["tab", "split", "window"])
    );
}

/// A decision model cannot run a subagent, and the spawn refuses one. The
/// schema says so where the model chooses a provider, naming each decision
/// provider id, so the model is told before it asks rather than after.
#[test]
fn the_provider_override_says_a_decision_model_is_refused() {
    let tool = create_subagent_tool(&[]);
    let description = tool.input_schema["properties"]["settings"]["properties"]["provider"]
        ["description"]
        .as_str()
        .expect("the provider override is described");

    assert!(
        description.contains("is refused"),
        "the provider override must say a decision model is refused: {description}"
    );
    for provider in ["baseten_decision", "ollama_decision"] {
        assert!(
            description.contains(provider),
            "the provider override must name `{provider}`: {description}"
        );
    }
}

/// The spawn schema an out-of-crate consumer sees offers `monitors`.
#[test]
fn the_spawn_schema_offers_monitors() {
    let tool = create_subagent_tool(&[]);
    let monitors = &tool.input_schema["properties"]["monitors"];
    assert_eq!(monitors["type"], "array", "{monitors}");
    assert_eq!(monitors["maxItems"], 4, "{monitors}");
    assert!(
        monitors["items"]["properties"].get("question").is_some(),
        "{monitors}"
    );
}

/// A parent whose only interesting property is that it never answers: every
/// spawn below is refused before a child could reach it.
struct NeverCalled;

#[async_trait::async_trait]
impl biorouter::providers::base::Provider for NeverCalled {
    fn metadata() -> biorouter::providers::base::ProviderMetadata {
        biorouter::providers::base::ProviderMetadata::new(
            "never-called",
            "Never called",
            "",
            "never-called-model",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "never-called"
    }

    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("never-called-model")
    }

    async fn complete_with_model(
        &self,
        _model_config: &biorouter::model::ModelConfig,
        _system: &str,
        _messages: &[biorouter::conversation::message::Message],
        _tools: &[rmcp::model::Tool],
    ) -> Result<
        (
            biorouter::conversation::message::Message,
            biorouter::providers::base::ProviderUsage,
        ),
        biorouter::providers::errors::ProviderError,
    > {
        panic!("a refused spawn never reaches a model");
    }
}

const CHECKER_WITH_A_PLAN: &str = r#"
version: "1.0.0"
title: "Checker"
description: "Checks the work in phases"
instructions: "Check the work in this folder."
orchestration_plan:
  title: Checking
  phases:
    - {id: read, title: Read the work}
    - {id: judge, title: Judge it, depends_on: [read]}
"#;

/// A sub-workflow that declares an `orchestration_plan` is refused at spawn,
/// with the sentence that says why and what to change, and no child row is
/// left behind: a subagent cannot run phases, because every workspace tool is
/// refused to it, so the plan would never be followed.
#[tokio::test]
async fn a_subworkflow_declaring_a_plan_is_refused_and_leaves_no_child() {
    use biorouter::session::session_manager::SessionType;
    use std::sync::Arc;

    let dir = TempDir::new().unwrap();
    let path = write_workflow(&dir, "checker", CHECKER_WITH_A_PLAN);
    let sm = Arc::new(biorouter::session::SessionManager::new(
        dir.path().to_path_buf(),
    ));
    let parent = sm
        .create_session(
            dir.path().to_path_buf(),
            "orchestrator".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    let config = biorouter::agents::AgentConfig::new(
        sm.clone(),
        Arc::new(biorouter::config::permission::PermissionManager::new(
            dir.path().to_path_buf(),
        )),
        None,
        biorouter::config::BioRouterMode::Auto,
    );
    let task_config =
        biorouter::agents::TaskConfig::new(Arc::new(NeverCalled), &parent.id, dir.path(), vec![]);
    let sub_workflows = HashMap::from([(
        "checker".to_string(),
        make_subworkflow(path, "checker", None),
    )]);

    let refused = biorouter::agents::subagent_tool::handle_subagent_tool(
        &config,
        serde_json::json!({ "subworkflow": "checker", "visible": false }),
        task_config,
        sub_workflows,
        dir.path().to_path_buf(),
        None,
    )
    .result
    .await
    .expect_err("a sub-workflow that declares a plan is refused");

    assert_eq!(refused.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        refused.message,
        "Could not spawn subworkflow \"checker\": subworkflow `checker` declares an \
         `orchestration_plan`, and a subagent cannot run phases: workspace tools are refused \
         to subagents. Remove the block from that file. Nothing was started."
    );
    assert!(
        sm.list_subagent_children(&parent.id)
            .await
            .unwrap()
            .is_empty(),
        "no SubAgent row"
    );
    assert_eq!(
        sm.count_all_sessions().await.unwrap(),
        1,
        "only the parent exists"
    );
}
