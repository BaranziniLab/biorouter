use crate::workflows::print_workflow::{
    missing_parameters_command_line, print_required_parameters_for_template,
    print_workflow_explanation,
};
use crate::workflows::search_workflow::load_workflow_file;
use crate::workflows::secret_discovery::{discover_workflow_secrets, SecretRequirement};
use anyhow::Result;
use biorouter::config::Config;
use biorouter::workflow::build_workflow::{
    apply_values_to_parameters, build_workflow_from_template, WorkflowError,
};
use biorouter::workflow::validate_workflow::{
    parse_and_validate_parameters, validate_workflow_template_from_content,
};
use biorouter::workflow::Workflow;

fn create_user_prompt_callback() -> impl Fn(&str, &str) -> Result<String> {
    |key: &str, description: &str| -> Result<String> {
        let input_value =
            cliclack::input(format!("Please enter {} ({})", key, description)).interact()?;
        Ok(input_value)
    }
}

pub fn load_workflow(workflow_name: &str, params: Vec<(String, String)>) -> Result<Workflow> {
    let workflow_file = load_workflow_file(workflow_name)?;
    let workflow_content = workflow_file.content;
    let workflow_dir = workflow_file.parent_dir;
    // R3-F5: a workflow the checks refuse is refused in `--explain`'s words.
    // Building it checks the same things, but reports them as "Template
    // rendering failed", which a workflow with no template problem never had.
    validate_workflow_template_from_content(
        &workflow_content,
        Some(workflow_dir.display().to_string()),
    )
    .map_err(|error| anyhow::anyhow!("{}", explain_refusal(&error.to_string())))?;
    match build_workflow_from_template(
        workflow_content,
        &workflow_dir,
        params,
        Some(create_user_prompt_callback()),
    ) {
        Ok(workflow) => {
            let secret_requirements = discover_workflow_secrets(&workflow);
            if let Err(e) = collect_missing_secrets(&secret_requirements) {
                eprintln!(
                    "Warning: Failed to collect some secrets: {}. Workflow will continue to run.",
                    e
                );
            }
            Ok(workflow)
        }
        Err(WorkflowError::MissingParams { parameters }) => Err(anyhow::anyhow!(
            "Please provide the following parameters in the command line: {}",
            missing_parameters_command_line(parameters)
        )),
        Err(e) => Err(anyhow::anyhow!(e.to_string())),
    }
}

/// Collects missing secrets from the user interactively
///
/// This function checks if each required secret exists in the keyring.
/// For missing secrets, it prompts the user interactively and stores them
/// using the scoped key to prevent collisions.
///
/// # Arguments
/// * `requirements` - Vector of SecretRequirement objects to collect
///
/// # Returns
/// Result indicating success or failure of the collection process
pub fn collect_missing_secrets(requirements: &[SecretRequirement]) -> Result<()> {
    if requirements.is_empty() {
        return Ok(());
    }

    let config = Config::global();
    let mut missing_secrets = Vec::new();

    for req in requirements {
        match config.get_secret::<String>(&req.key) {
            Ok(_) => continue, // Secret exists
            Err(_) => missing_secrets.push(req),
        }
    }

    if missing_secrets.is_empty() {
        return Ok(());
    }

    println!(
        "🔐 This workflow uses {} secret(s) that are not yet configured (press ESC to skip any that are optional):",
        missing_secrets.len()
    );

    for req in &missing_secrets {
        println!("\n📋 Extension: {}", req.extension_name);
        println!("🔑 Secret: {}", req.key);

        let value = cliclack::password(format!(
            "Enter {} ({}) - press ESC to skip",
            req.key,
            req.description()
        ))
        .mask('▪')
        .interact()
        .unwrap_or_else(|_| String::new());

        if !value.trim().is_empty() {
            if let Err(e) = config.set_secret(&req.key, &value) {
                println!("⚠️  Failed to store secret in secure storage: {}. Secret available for this run only.", e);
                println!(
                    "   Consider setting {} as an environment variable for future use.",
                    req.key
                );
            } else {
                println!("✅ Secret stored securely for {}", req.extension_name);
            }
        } else {
            println!("⏭️  Skipped {} for {}", req.key, req.extension_name);
        }
    }

    if !missing_secrets.is_empty() {
        println!("\n🎉 Secret collection complete! Workflow execution will now continue.");
    }

    Ok(())
}

pub fn render_workflow_as_yaml(workflow_name: &str, params: Vec<(String, String)>) -> Result<()> {
    let workflow = load_workflow(workflow_name, params)?;
    // The writer every saved workflow goes through, so a question's kind
    // reads `{binary: {}}` here too, never a `!binary` tag (R2-8).
    match workflow.to_yaml() {
        Ok(yaml_content) => {
            println!("{}", yaml_content);
            Ok(())
        }
        Err(_) => {
            eprintln!("Failed to serialize workflow to YAML");
            std::process::exit(1);
        }
    }
}

pub fn explain_workflow(workflow_name: &str, params: Vec<(String, String)>) -> Result<()> {
    let workflow_file = load_workflow_file(workflow_name)?;
    let workflow_dir_str = workflow_file.parent_dir.display().to_string();
    let workflow_file_content = &workflow_file.content;
    let workflow_template =
        parse_and_validate_parameters(workflow_file_content, Some(workflow_dir_str.clone()))?;
    // H-3: the same checks `run` and the desktop's import make, so a
    // workflow `--explain` describes is one that would start. Whether each
    // decision model is there and allowed is only known when the run starts,
    // which the Supervision section says.
    validate_workflow_template_from_content(workflow_file_content, Some(workflow_dir_str.clone()))
        .map_err(|error| anyhow::anyhow!("{}", explain_refusal(&error.to_string())))?;
    let workflow_parameters = workflow_template.parameters.clone();

    let (params_for_template, missing_params) = apply_values_to_parameters(
        &params,
        workflow_parameters,
        &workflow_dir_str,
        None::<fn(&str, &str) -> Result<String>>,
    )?;
    print_workflow_explanation(&workflow_template);
    print_required_parameters_for_template(params_for_template, missing_params);

    Ok(())
}

/// What `--explain` says of a workflow `run` would refuse: the refusal `run`
/// gives, with its models named as the terminal names them and the advice a
/// person follows.
fn explain_refusal(refusal: &str) -> String {
    format!(
        "This workflow would not start: {}",
        crate::commands::decide::terminal_advice(
            &crate::commands::decide::person_model_names_in_sentence(refusal)
        )
    )
}

#[cfg(test)]
mod tests {
    use biorouter::workflow::{WorkflowParameterInputType, WorkflowParameterRequirement};

    use crate::workflows::workflow::{explain_workflow, load_workflow};

    /// H-3: `--explain` refuses what `run` and the import refuse, in the
    /// same words, instead of describing a workflow that would not start.
    #[test]
    fn explain_refuses_a_workflow_run_would_refuse() {
        const VALID: &str = "title: Cohort screen\ndescription: Screen notes\nprompt: Screen \
             them.\norchestration_plan:\n  title: Screen and verify\n  phases:\n    - {id: \
             screen, title: Screen the notes, kind: research}\n    - id: verify\n      title: \
             Verify\n      kind: verification\n      gate:\n        questions:\n          \
             counted: Does the hand-off give the cohort size?\n        threshold: 0.8\n";
        let dir = tempfile::tempdir().unwrap();
        let explain = |name: &str, content: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            explain_workflow(path.to_str().unwrap(), Vec::new())
        };
        explain("valid.yaml", VALID).expect("a valid workflow is explained");
        for (name, content, expected) in [
            (
                "threshold.yaml",
                VALID.replace("threshold: 0.8", "threshold: 1.7"),
                "gate threshold must be a probability from 0 to 1",
            ),
            (
                "missing.yaml",
                VALID.replace(
                    "kind: verification",
                    "kind: verification\n      depends_on: [nosuch]",
                ),
                "Phase \"verify\" depends on \"nosuch\"",
            ),
            (
                "twice.yaml",
                VALID.replace(
                    "    - id: verify",
                    "    - {id: screen, title: Again, kind: research}\n    - id: verify",
                ),
                "Phase id \"screen\" appears twice",
            ),
        ] {
            let error = explain(name, &content)
                .expect_err("run refuses it, so --explain does")
                .to_string();
            assert!(
                error.starts_with("This workflow would not start: "),
                "{name}: {error}"
            );
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    /// R3-F5: `run` refuses what `--explain` refuses in the same words,
    /// never as "Template rendering failed".
    #[test]
    fn run_refuses_a_workflow_in_explains_words() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threshold.yaml");
        std::fs::write(
            &path,
            "title: Cohort screen\ndescription: Screen notes\nprompt: Screen them.\n\
             orchestration_plan:\n  title: Screen\n  phases:\n    - id: verify\n      \
             title: Verify\n      kind: verification\n      gate:\n        questions:\n          \
             counted: Does the hand-off give the cohort size?\n        threshold: 1.7\n",
        )
        .unwrap();
        let error = load_workflow(path.to_str().unwrap(), Vec::new())
            .expect_err("a threshold of 1.7 is refused")
            .to_string();
        let explained = explain_workflow(path.to_str().unwrap(), Vec::new())
            .expect_err("--explain refuses it too")
            .to_string();
        assert!(!error.contains("Template rendering failed"), "{error}");
        assert!(
            error.starts_with("This workflow would not start: "),
            "{error}"
        );
        assert_eq!(error, explained);
    }

    mod load_workflow {
        use super::*;
        #[test]
        fn test_load_workflow_success() {
            let workflow_content = r#"{
                "version": "1.0.0",
                "title": "Test Workflow",
                "description": "A test workflow",
                "instructions": "Test instructions with {{ my_name }}",
                "parameters": [
                    {
                        "key": "my_name",
                        "input_type": "string",
                        "requirement": "required",
                        "description": "A test parameter"
                    }
                ]
            }"#;
            let temp_dir = tempfile::tempdir().unwrap();
            let workflow_path = temp_dir.path().join("test_workflow.json");
            std::fs::write(&workflow_path, workflow_content).unwrap();

            let params = vec![("my_name".to_string(), "value".to_string())];
            let workflow = load_workflow(workflow_path.to_str().unwrap(), params).unwrap();

            assert_eq!(workflow.title, "Test Workflow");
            assert_eq!(workflow.description, "A test workflow");
            assert_eq!(
                workflow.instructions.unwrap(),
                "Test instructions with value"
            );
            // Verify parameters match workflow definition
            assert_eq!(workflow.parameters.as_ref().unwrap().len(), 1);
            let param = &workflow.parameters.as_ref().unwrap()[0];
            assert_eq!(param.key, "my_name");
            assert!(matches!(
                param.input_type,
                WorkflowParameterInputType::String
            ));
            assert!(matches!(
                param.requirement,
                WorkflowParameterRequirement::Required
            ));
            assert_eq!(param.description, "A test parameter");
        }
    }
}
