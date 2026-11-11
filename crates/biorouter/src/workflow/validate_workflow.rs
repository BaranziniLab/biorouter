use crate::workflow::read_workflow_file_content::WorkflowFile;
use crate::workflow::supervision;
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
    validate_builtin_extensions(&workflow)?;
    // The monitors and the phase plan the workflow declares, checked here so
    // every door that validates a workflow (the CLI, the import, the HTTP
    // routes, the agent's `manage_workflow`) refuses what a chat could not
    // take.
    supervision::validate(&workflow).map_err(anyhow::Error::msg)?;
    if let Some(response) = &workflow.response {
        if let Some(json_schema) = &response.json_schema {
            validate_json_schema(json_schema)?;
        }
    }

    Ok(workflow)
}

/// H-5: a `type: builtin` extension must name one Biorouter ships. One that
/// does not used to be accepted here and then skipped at run time with a
/// warning, which left an unexplained "failed to load" notice in every chat
/// the workflow ran in. The name is matched exactly as the extension manager
/// looks it up when it starts the extension; a platform capability written as
/// a built-in (`workspace`, the drafter's usual slip) is told how to write it.
fn validate_builtin_extensions(workflow: &Workflow) -> Result<()> {
    use crate::agents::extension::{ExtensionConfig, PLATFORM_EXTENSIONS};
    let key = |name: &str| {
        crate::agents::extension_manager::normalize(&crate::config::extensions::name_to_key(name))
    };
    for extension in workflow.extensions.iter().flatten() {
        let ExtensionConfig::Builtin { name, .. } = extension else {
            continue;
        };
        // Exactly the lookup the extension manager starts a built-in with.
        if biorouter_mcp::BUILTIN_EXTENSIONS.contains_key(name.as_str()) {
            continue;
        }
        let mut known: Vec<&str> = biorouter_mcp::BUILTIN_EXTENSIONS.keys().copied().collect();
        known.sort_unstable();
        let platform = PLATFORM_EXTENSIONS
            .keys()
            .find(|platform| key(platform) == key(name))
            .map(|platform| {
                format!(
                    " `{platform}` is a platform capability: write it as `type: platform`, \
                     `name: {platform}`."
                )
            })
            .unwrap_or_default();
        return Err(anyhow::anyhow!(
            "The workflow names a built-in extension `{}` that Biorouter does not have. \
             Built-in extensions: {}.{platform}",
            name.trim(),
            known.join(", ")
        ));
    }
    Ok(())
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

    /// H-5: an unknown built-in extension is refused here, naming the known
    /// ones, and a platform capability written as a built-in is told how to
    /// write it.
    #[test]
    fn an_unknown_builtin_extension_is_refused_with_the_known_names() {
        let file = |name: &str, kind: &str| {
            format!(
                "title: Fan out\ndescription: d\nprompt: Do the work.\nextensions:\n  - type: {kind}\n    name: {name}\n"
            )
        };
        let refused = validate_workflow_template_from_content(&file("workspace", "builtin"), None)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("built-in extension `workspace` that Biorouter does not have"),
            "{refused}"
        );
        assert!(refused.contains("developer"), "{refused}");
        assert!(
            refused.contains("write it as `type: platform`"),
            "{refused}"
        );

        let refused = validate_workflow_template_from_content(&file("nosuch", "builtin"), None)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("`nosuch`"), "{refused}");
        assert!(!refused.contains("platform capability"), "{refused}");

        // Started by exact name, so a different spelling would not start either.
        assert!(
            validate_workflow_template_from_content(&file("Developer", "builtin"), None).is_err()
        );
        for accepted in [file("developer", "builtin"), file("workspace", "platform")] {
            validate_workflow_template_from_content(&accepted, None)
                .unwrap_or_else(|e| panic!("{accepted}: {e}"));
        }
    }

    /// A workflow's monitors and phase plan are validated with the rest of the
    /// file, so `biorouter workflow validate`, the import and the routes refuse
    /// what a chat could not take.
    #[test]
    fn supervision_blocks_are_validated_with_the_workflow() {
        let file = |threshold: &str| {
            format!(
                r#"
title: Watched
description: Watched work
prompt: Do the work.
decision_settings:
  monitors:
    - id: off-task
      name: Off task
      question: {{instructions: Is the agent off task?, kind: {{binary: {{}}}}}}
      threshold: {threshold}
      action: stop
      onError: pause
orchestration_plan:
  title: Work
  phases:
    - {{id: work, title: Work}}
"#
            )
        };
        let workflow = validate_workflow_template_from_content(&file("0.8"), None).unwrap();
        assert!(workflow.decision_settings.is_some());
        assert!(workflow.orchestration_plan.is_some());
        let error = validate_workflow_template_from_content(&file("1.5"), None)
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(
                "The workflow's `decision_settings`: Monitor \"Off task\": the threshold must be \
                 a probability above 0 and at most 1"
            ),
            "{error}"
        );
    }

    /// The parameter checks run first, so a `{{ topic }}` in a monitor's
    /// question reaches the supervision refusal only once `topic` is declared
    /// under `parameters:`. Undeclared, it is refused as a missing parameter,
    /// as it is without these blocks.
    #[test]
    fn a_parameter_in_a_monitor_is_refused_after_the_parameter_checks() {
        let file = |parameters: &str| {
            format!(
                r#"
title: Watched
description: Watched work
prompt: Do the work.
decision_settings:
  monitors:
    - id: reached-ten
      name: Reached ten
      question:
        instructions: Has the agent already echoed {{{{ topic }}}}?
        kind: {{binary: {{}}}}
      threshold: 0.7
      action: stop
      onError: pause
{parameters}"#
            )
        };
        let undeclared = validate_workflow_template_from_content(&file(""), None)
            .unwrap_err()
            .to_string();
        assert_eq!(
            undeclared,
            "Missing definitions for parameters in the workflow file: topic."
        );

        let declared = file(
            "parameters:\n  - key: topic\n    input_type: string\n    requirement: required\n    \
             description: what to count\n",
        );
        let error = validate_workflow_template_from_content(&declared, None)
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(
                "The workflow's `decision_settings` and `orchestration_plan` cannot use \
                 parameters (`{{ topic }}` in monitor `reached-ten`)"
            ),
            "{error}"
        );
    }
}
