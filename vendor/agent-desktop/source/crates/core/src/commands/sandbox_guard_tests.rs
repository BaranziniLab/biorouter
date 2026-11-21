use super::execute;
use crate::commands::sandbox_action::SandboxAction;
use crate::refs::refs_test_support::HomeGuard;
use crate::sandbox::provider_test_support::ScriptedAdapter;
use crate::sandbox::sandbox_record::SandboxRecord;
use crate::sandbox::sandbox_store;
use crate::{AdapterError, AppError, DeliverySemantics, ErrorCode};
use std::path::PathBuf;

const STOPPED: &str = r#"{"ok":true,"state":"stopped","message":"Sandbox dev-1 is stopped."}"#;
const REFUSED: &str = r#"{"ok":false,"code":"INVALID_ARGS","message":"The provider refused.","suggestion":"Fix the settings."}"#;

fn sandboxes_root(home: &HomeGuard) -> PathBuf {
    home.path().join(".agent-desktop").join("sandboxes")
}

fn publish(home: &HomeGuard) -> PathBuf {
    let record = SandboxRecord {
        name: "dev-1".to_string(),
        provider: "stub".to_string(),
        created_at: 1,
    };
    sandbox_store::create(&sandboxes_root(home), &record).expect("record")
}

fn create_action() -> SandboxAction {
    SandboxAction::Create {
        name: "dev-1".to_string(),
        provider: "stub".to_string(),
        set: Vec::new(),
    }
}

fn status_action() -> SandboxAction {
    SandboxAction::Status {
        name: "dev-1".to_string(),
    }
}

fn disposition(error: AppError) -> DeliverySemantics {
    match error {
        AppError::Adapter(error) => error.disposition,
        other => panic!("expected an adapter error, got {other:?}"),
    }
}

#[test]
fn a_broken_record_is_listed_with_the_directory_to_delete_and_reaches_no_provider() {
    let home = HomeGuard::new();
    let dir = publish(&home);
    std::fs::write(dir.join(sandbox_store::RECORD_FILE), b"{not json").expect("corrupt record");
    let adapter = ScriptedAdapter::new();

    let listed = execute(SandboxAction::List, &adapter).expect("list");
    let entry = &listed["sandboxes"][0];
    assert_eq!(entry["state"], "broken");
    let next = entry["next"].as_str().unwrap_or_default();
    assert!(next.contains(&dir.display().to_string()), "next was {next}");

    for action in [
        status_action(),
        SandboxAction::Rm {
            name: "dev-1".to_string(),
        },
    ] {
        let error = execute(action, &adapter).expect_err("a broken record is refused");
        assert_eq!(error.suggestion(), Some(next));
    }
    assert!(adapter.requests().is_empty(), "the provider never ran");
    assert!(dir.exists());
}

#[test]
fn create_does_not_adopt_a_directory_that_holds_another_sandboxs_data() {
    let home = HomeGuard::new();
    let dir = sandboxes_root(&home).join("dev-1");
    crate::storage::private_file_parent::ensure_private(&dir).expect("sandbox directory");
    std::fs::write(dir.join("disk.img"), b"data").expect("provider data");
    let adapter = ScriptedAdapter::new();

    let error = execute(create_action(), &adapter).expect_err("the directory is taken");

    assert_eq!(error.code(), "INVALID_ARGS");
    assert!(adapter.requests().is_empty(), "the provider never ran");
    assert!(!dir.join(sandbox_store::RECORD_FILE).exists());
    assert!(dir.join("disk.img").exists());
}

#[test]
fn a_refused_create_that_drops_its_record_reports_nothing_delivered() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new().reply("create", REFUSED, 1);

    let error = execute(create_action(), &adapter).expect_err("the provider refuses");

    assert!(!sandboxes_root(&home).join("dev-1").exists());
    assert_eq!(disposition(error), DeliverySemantics::not_delivered());
}

#[test]
fn a_command_does_not_change_a_sandbox_another_command_is_changing() {
    let home = HomeGuard::new();
    publish(&home);
    let _held = sandbox_store::hold(&sandboxes_root(&home), "dev-1").expect("first holder");
    let adapter = ScriptedAdapter::new().reply("status", STOPPED, 0);

    for action in [
        SandboxAction::Rm {
            name: "dev-1".to_string(),
        },
        SandboxAction::Start {
            name: "dev-1".to_string(),
            set: Vec::new(),
        },
        create_action(),
    ] {
        let error = execute(action, &adapter).expect_err("the sandbox is busy");
        assert_eq!(disposition(error), DeliverySemantics::not_delivered());
    }

    assert!(adapter.requests().is_empty(), "the provider never ran");
    execute(status_action(), &adapter).expect("reads are not blocked");
}

#[test]
fn list_passes_on_what_a_failed_status_suggests_instead_of_advising_removal() {
    let home = HomeGuard::new();
    publish(&home);
    let timeout = AdapterError::new(ErrorCode::Timeout, "The provider timed out.")
        .with_suggestion("Run `agent-desktop sandbox status dev-1`.");
    let adapter = ScriptedAdapter::new().fail("status", timeout);

    let listed = execute(SandboxAction::List, &adapter).expect("list");

    let next = listed["sandboxes"][0]["next"].as_str().unwrap_or_default();
    assert!(next.contains("sandbox status dev-1"), "next was {next}");
    assert!(!next.contains("sandbox rm"), "next was {next}");
}
