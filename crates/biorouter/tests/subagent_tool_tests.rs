use biorouter::agents::subagent_tool::{create_subagent_tool, SUBAGENT_TOOL_NAME};
use biorouter::workflow::{SubWorkflow, Workflow};
use std::collections::HashMap;
use tempfile::TempDir;

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
