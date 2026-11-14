biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::workflow::{build_workflow::build_workflow_from_template, Workflow};
use serde_json::Value;
use std::path::Path;

fn render_workflow(extra: &[(&str, &str)]) -> Workflow {
    let mut parameters = vec![
        ("test_phases".to_owned(), "apps,previews".to_owned()),
        ("parallel_tests".to_owned(), "false".to_owned()),
        ("cleanup_after".to_owned(), "false".to_owned()),
        ("workspace_dir".to_owned(), "/tmp/selftest-qa".to_owned()),
    ];
    for (key, value) in extra {
        parameters.retain(|(existing, _)| existing.as_str() != *key);
        parameters.push(((*key).to_owned(), (*value).to_owned()));
    }
    let workflow = build_workflow_from_template(
        include_str!("../../../biorouter-self-test.yaml").to_owned(),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        parameters,
        None::<fn(&str, &str) -> anyhow::Result<String>>,
    )
    .expect("self-test workflow renders without credentials or provider calls");
    workflow
}

fn render_prompt(extra: &[(&str, &str)]) -> String {
    let workflow = render_workflow(extra);
    format!(
        "{}\n{}",
        workflow.instructions.expect("self-test has instructions"),
        workflow.prompt.expect("self-test has an execution prompt")
    )
}

fn rendered_extension_names(workflow: &Workflow) -> Vec<String> {
    workflow
        .extensions
        .as_ref()
        .expect("self-test has rendered extensions")
        .iter()
        .map(|extension| {
            serde_json::to_value(extension)
                .expect("extension config serializes")
                .get("name")
                .and_then(Value::as_str)
                .expect("extension config has a name")
                .to_owned()
        })
        .collect()
}

#[test]
fn cleanup_disabled_never_instructs_app_deletion_or_collision_replacement() {
    let prompt = render_prompt(&[]);
    assert!(!prompt.contains("`delete_app`"));
    assert!(!prompt.contains("Delete only the"));
    assert!(!prompt.contains("delete only it"));
    assert_eq!(
        prompt
            .matches("stop this phase and report the collision")
            .count(),
        2
    );
}

#[test]
fn distinct_app_parameters_reach_create_launch_and_preview_contracts() {
    let prompt = render_prompt(&[
        ("sdk_app_id", "qa-sdk-unique"),
        ("preview_app_id", "qa-preview-unique"),
    ]);
    assert!(prompt.contains("exact app id `qa-sdk-unique`"));
    assert!(prompt.contains("exact app id `qa-preview-unique`"));
    assert!(prompt.contains("id `qa-preview-unique`"));
    assert!(prompt.contains("`/apps/qa-preview-unique/`"));
    assert!(!prompt.contains("biorouter-self-test-sdk-v2"));
    assert!(!prompt.contains("biorouter-self-test-preview"));
}

#[test]
fn explicit_cleanup_is_limited_to_apps_created_by_this_run() {
    let prompt = render_prompt(&[("cleanup_after", "true")]);
    assert_eq!(prompt.matches("`delete_app`").count(), 2);
    assert_eq!(prompt.matches("only if this run created it").count(), 2);
    assert!(!prompt.contains("delete only it"));
}

#[test]
fn crew_phase_is_selectable_and_headless() {
    let workflow = render_workflow(&[("test_phases", "crew")]);
    let extensions = rendered_extension_names(&workflow);
    assert!(extensions.contains(&"developer".to_owned()));
    assert!(extensions.contains(&"crew".to_owned()));
    assert!(!extensions.contains(&"computercontroller".to_owned()));
    assert!(!extensions.contains(&"webdocuments".to_owned()));

    let all_workflow = render_workflow(&[("test_phases", "all")]);
    let all_extensions = rendered_extension_names(&all_workflow);
    assert!(all_extensions.contains(&"crew".to_owned()));
    assert!(all_extensions.contains(&"computercontroller".to_owned()));

    let prompt = render_prompt(&[("test_phases", "crew")]);
    assert!(prompt.contains("PHASE 2E: Hidden Crew CLI without tool invocation"));
    assert!(prompt.contains("/no_think"));
    assert!(prompt.contains("`hidden_cli_no_invocation_v1`"));
    assert!(prompt.contains("--no-session --with-builtin crew"));
    assert!(prompt.contains("CREW_STANDALONE_NO_INVOCATION_OK"));
    assert!(prompt.contains("The harness did not supply all five handles."));
    assert!(prompt.contains("<biorouterd> --version"));
    assert!(!prompt.contains("PHASE 2D: Native Biorouter Copilot"));
    assert!(!prompt.contains("computer_use_fixture"));

    let configured = render_prompt(&[
        ("test_phases", "crew"),
        ("crew_cli_path", "/tmp/selftest-qa/bin/biorouter"),
        ("crew_standalone_checker", "/tmp/selftest-qa/checker.py"),
        ("crew_standalone_profile_root", "/tmp/selftest-qa/provider"),
        ("crew_standalone_provider", "synthetic-provider"),
        ("crew_standalone_model", "synthetic-model"),
    ]);
    for marker in [
        "python3 /tmp/selftest-qa/checker.py --cli /tmp/selftest-qa/bin/biorouter",
        "--profile-root /tmp/selftest-qa/provider --provider synthetic-provider",
        "--model synthetic-model",
        "contract=hidden_cli_no_invocation_v1",
        "tool_request_count=0",
        "tool_response_count=0",
        "fixed_response=true",
        "completed=true",
        "channel_content_returned=false",
        "effective_model_roster=NOT OBSERVABLE",
        "forged_call_denial=NOT OBSERVABLE",
        "separate_rust_evidence_required=true",
    ] {
        assert!(
            configured.contains(marker),
            "missing Crew contract marker: {marker}"
        );
    }
    assert!(!configured.contains("`request_count=1`"));
    assert!(!configured.contains("`exact_denial=true`"));
}

#[test]
fn preview_only_does_not_render_the_dashboard_phase() {
    let prompt = render_prompt(&[("test_phases", "previews")]);
    assert!(!prompt.contains("exact app id `biorouter-self-test-sdk-v2`"));
    assert!(prompt.contains("exact app id `biorouter-self-test-preview`"));
    assert!(!prompt.contains("`delete_app`"));
}
