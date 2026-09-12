use crate::workflow::read_workflow_file_content::WorkflowFile;
use crate::workflow::template_workflow::parse_workflow_content;
use crate::workflow::{
    Workflow, WorkflowParameter, WorkflowParameterInputType, WorkflowParameterRequirement,
    BUILT_IN_WORKFLOW_DIR_PARAM,
};
use anyhow::Result;
use std::collections::HashSet;

pub fn parse_and_validate_parameters(
    workflow_file_content: &str,
    workflow_dir_str: Option<String>,
) -> Result<Workflow> {
    let (workflow_template, template_variables) =
        parse_workflow_content(workflow_file_content, workflow_dir_str)?;
    let workflow_parameters = &workflow_template.parameters;
    validate_optional_parameters(workflow_parameters)?;
    validate_parameters_in_template(workflow_parameters, &template_variables)?;
    Ok(workflow_template)
}

fn validate_json_schema(schema: &serde_json::Value) -> Result<()> {
    match jsonschema::validator_for(schema) {
        Ok(_) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("JSON schema validation failed: {}", err)),
    }
}

pub fn validate_workflow_template_from_file(workflow_file: &WorkflowFile) -> Result<Workflow> {
    let workflow_dir = workflow_file
        .parent_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Error getting workflow directory"))?
        .to_string();

    validate_workflow_template_from_content(&workflow_file.content, Some(workflow_dir))
}

pub fn validate_workflow_template_from_content(
    workflow_content: &str,
    workflow_dir: Option<String>,
) -> Result<Workflow> {
    parse_and_validate_parameters(workflow_content, workflow_dir.clone())?;
    let (workflow, _) = parse_workflow_content(workflow_content, workflow_dir)?;

    validate_prompt_or_instructions(&workflow)?;
    if let Some(response) = &workflow.response {
        if let Some(json_schema) = &response.json_schema {
            validate_json_schema(json_schema)?;
        }
    }

    Ok(workflow)
}

fn validate_prompt_or_instructions(workflow: &Workflow) -> Result<()> {
    let has_instructions = workflow
        .instructions
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let has_prompt = workflow
        .prompt
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    if has_instructions || has_prompt {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "Workflow must specify at least one of `instructions` or `prompt`."
    ))
}

fn validate_parameters_in_template(
    workflow_parameters: &Option<Vec<WorkflowParameter>>,
    template_variables: &HashSet<String>,
) -> Result<()> {
    let mut template_variables = template_variables.clone();
    template_variables.remove(BUILT_IN_WORKFLOW_DIR_PARAM);

    let param_keys: HashSet<String> = workflow_parameters
        .as_ref()
        .unwrap_or(&vec![])
        .iter()
        .map(|p| p.key.clone())
        .collect();

    let missing_keys = template_variables
        .difference(&param_keys)
        .collect::<Vec<_>>();

    let extra_keys = param_keys
        .difference(&template_variables)
        .collect::<Vec<_>>();

    if missing_keys.is_empty() && extra_keys.is_empty() {
        return Ok(());
    }

    let mut message = String::new();

    if !missing_keys.is_empty() {
        // Sorted, like the arm below: both sides come out of a `HashSet`
        // difference, so an unsorted join gives the same fault a different
        // message on each run.
        let mut names: Vec<String> = missing_keys.iter().map(|s| s.to_string()).collect();
        names.sort();
        message.push_str(&format!(
            "Missing definitions for parameters in the workflow file: {}.",
            names.join(", ")
        ));
    }

    if !extra_keys.is_empty() {
        // The prefix is load-bearing — it is what the messages users and tests
        // have already matched on say. What follows it is new: the old message
        // named the parameter and stopped, leaving "unnecessary" to be guessed
        // at, and the obvious guess (remove its `default`) trades this refusal
        // for `validate_optional_parameters`' one. Naming the fix rather than
        // only the fault is the difference between a rule and a dead end.
        let mut names: Vec<String> = extra_keys.iter().map(|s| s.to_string()).collect();
        names.sort();
        message.push_str(&format!(
            "\nUnnecessary parameter definitions: {}. Nothing in the workflow \
             refers to {}, so a value for it would be collected and then \
             discarded — write {{{{ {} }}}} into `prompt` or `instructions`, or \
             remove the definition. Giving it a `default` does not help.",
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" },
            names[0]
        ));
    }
    Err(anyhow::anyhow!("{}", message.trim_end()))
}

fn validate_optional_parameters(parameters: &Option<Vec<WorkflowParameter>>) -> Result<()> {
    let empty_params = vec![];
    let params = parameters.as_ref().unwrap_or(&empty_params);

    let file_params_with_defaults: Vec<String> = params
        .iter()
        .filter(|p| matches!(p.input_type, WorkflowParameterInputType::File) && p.default.is_some())
        .map(|p| p.key.clone())
        .collect();

    if !file_params_with_defaults.is_empty() {
        return Err(anyhow::anyhow!("File parameters cannot have default values to avoid importing sensitive user files: {}", file_params_with_defaults.join(", ")));
    }

    let optional_params_without_default_values: Vec<String> = params
        .iter()
        .filter(|p| {
            matches!(p.requirement, WorkflowParameterRequirement::Optional) && p.default.is_none()
        })
        .map(|p| p.key.clone())
        .collect();

    if optional_params_without_default_values.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Optional parameters missing default values in the workflow: {}. Please provide defaults.", optional_params_without_default_values.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_workflow_template_from_content_success() {
        let workflow_content = r#"
version: 1.0.0
title: Test Workflow
description: A test workflow for validation
instructions: Test instructions with {{ user_role }}
prompt: |
  {% if user_role in ["Director, Account Management", "Senior Director, Account Management"] %}
  - Focus on strategic planning and organizational performance
  {% else %}
  - Provide foundational account management guidance
  {% endif %}
parameters:
  - key: user_role
    input_type: string
    requirement: required
    description: A test parameter
"#;

        let result = validate_workflow_template_from_content(workflow_content, None);
        if let Err(e) = &result {
            eprintln!("Validation error: {}", e);
            eprintln!("Error chain:");
            let mut source = e.source();
            while let Some(err) = source {
                eprintln!("  Caused by: {}", err);
                source = err.source();
            }
        }
        assert!(result.is_ok(), "Validation failed: {:?}", result.err());

        let workflow = result.unwrap();
        assert_eq!(workflow.title, "Test Workflow");
        assert_eq!(workflow.description, "A test workflow for validation");
        assert!(workflow.instructions.is_some());
        println!("Workflow: {:?}", workflow.prompt);
    }
}
