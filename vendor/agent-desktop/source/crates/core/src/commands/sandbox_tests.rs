use super::execute;
use crate::commands::sandbox_action::SandboxAction;
use crate::refs::refs_test_support::HomeGuard;
use crate::sandbox::provider_test_support::ScriptedAdapter;
use crate::sandbox::sandbox_store;
use crate::{AdapterError, AppError, DeliverySemantics};
use std::path::PathBuf;

const CREATED: &str = r#"{"ok":true,"state":"stopped","message":"Sandbox dev-1 is created.","next":"Run `agent-desktop sandbox start dev-1`."}"#;
const STOPPED: &str = r#"{"ok":true,"state":"stopped","message":"Sandbox dev-1 is stopped."}"#;
const RUNNING: &str = r#"{"ok":true,"state":"running","message":"Sandbox dev-1 is running."}"#;
const REMOVED: &str = r#"{"ok":true,"state":"stopped","message":"Sandbox dev-1 is removed."}"#;
const UNAVAILABLE: &str =
    r#"{"ok":true,"state":"unavailable","message":"This Mac cannot run virtual machines."}"#;
const UNREADABLE: &str = "the provider printed no reply";
const RM_FAILED: &str = r#"{"ok":false,"code":"ACTION_FAILED","message":"The provider did not remove the sandbox.","suggestion":"Run `agent-desktop sandbox rm dev-1` again."}"#;

fn rm_action() -> SandboxAction {
    SandboxAction::Rm {
        name: "dev-1".to_string(),
    }
}

fn adapter_error(error: AppError) -> AdapterError {
    match error {
        AppError::Adapter(error) => error,
        other => panic!("expected an adapter error, got {other:?}"),
    }
}

fn create_action() -> SandboxAction {
    SandboxAction::Create {
        name: "dev-1".to_string(),
        provider: "stub".to_string(),
        set: Vec::new(),
    }
}

fn sandboxes_root(home: &HomeGuard) -> PathBuf {
    home.path().join(".agent-desktop").join("sandboxes")
}

fn env_value(request: &crate::ProviderScriptRequest, key: &str) -> Option<String> {
    request
        .env
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

#[test]
fn provider_calls_carry_the_callers_working_directory() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new().reply("create", CREATED, 0);
    let caller = std::env::current_dir().expect("the caller working directory");

    execute(create_action(), &adapter).expect("create");

    let requests = adapter.requests();
    let request = requests.first().expect("the provider create ran");
    assert_eq!(
        request.working_dir,
        sandboxes_root(&home).join("dev-1"),
        "the provider still runs inside the sandbox it owns"
    );
    assert_eq!(
        env_value(request, "AD_CALLER_CWD").as_deref(),
        Some(caller.to_str().expect("a utf-8 caller directory")),
        "the provider sees the caller directory it was invoked from, not its own"
    );
}

#[test]
fn a_failed_create_leaves_no_record_unless_the_provider_left_files() {
    {
        let home = HomeGuard::new();
        let adapter = ScriptedAdapter::new();

        execute(create_action(), &adapter).expect_err("the provider create fails");

        assert!(
            !sandboxes_root(&home).join("dev-1").exists(),
            "a failed create leaves no sandbox behind"
        );
    }

    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new().leaving("id_ed25519");

    execute(create_action(), &adapter).expect_err("the provider create fails");

    assert!(
        sandbox_store::load(&sandboxes_root(&home), "dev-1").is_ok(),
        "the record stays so status can report what the provider left"
    );
}

#[test]
fn a_create_the_provider_did_not_refuse_keeps_its_record_for_status_and_rm() {
    let timed_out = ScriptedAdapter::new().fail(
        "create",
        crate::AdapterError::timeout("the provider did not answer"),
    );
    let unreadable = ScriptedAdapter::new().reply("create", "the provider crashed", 1);
    let flooded = ScriptedAdapter::new().fail(
        "create",
        crate::AdapterError::new(
            crate::ErrorCode::ActionFailed,
            "the provider printed too much",
        ),
    );
    let incomplete_success = ScriptedAdapter::new().reply("create", r#"{"ok":true}"#, 0);
    let changed_then_failed = ScriptedAdapter::new().reply(
        "create",
        r#"{"ok":false,"code":"ACTION_FAILED","message":"The disk filled up.","suggestion":"Free space.","data":{"changed":true}}"#,
        1,
    );

    for adapter in [
        timed_out,
        unreadable,
        flooded,
        incomplete_success,
        changed_then_failed,
    ] {
        let home = HomeGuard::new();

        execute(create_action(), &adapter).expect_err("the create outcome is unknown");

        assert!(
            sandbox_store::load(&sandboxes_root(&home), "dev-1").is_ok(),
            "the record stays so the sandbox still lists and can be removed"
        );
    }
}

#[test]
fn create_on_a_provider_that_cannot_run_here_is_refused_and_leaves_no_record() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new().reply("create", UNAVAILABLE, 0);

    let error = execute(create_action(), &adapter).expect_err("the provider is unavailable");

    assert_eq!(error.code(), "PLATFORM_NOT_SUPPORTED");
    assert!(
        !sandboxes_root(&home).join("dev-1").exists(),
        "an unavailable provider leaves no sandbox behind"
    );
}

#[test]
fn removing_a_running_sandbox_is_refused_and_keeps_the_record() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new()
        .reply("create", CREATED, 0)
        .reply("status", RUNNING, 0);
    execute(create_action(), &adapter).expect("create");

    let error =
        adapter_error(execute(rm_action(), &adapter).expect_err("a running sandbox is refused"));

    assert_eq!(error.code.as_str(), "INVALID_ARGS");
    assert_eq!(error.disposition, DeliverySemantics::not_delivered());
    assert_eq!(adapter.requests().len(), 2, "the rm verb never runs");
    assert!(
        sandbox_store::load(&sandboxes_root(&home), "dev-1").is_ok(),
        "the record stays"
    );
}

#[test]
fn removing_a_stopped_sandbox_removes_the_record() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new()
        .reply("create", CREATED, 0)
        .reply("status", STOPPED, 0)
        .reply("rm", REMOVED, 0);
    execute(create_action(), &adapter).expect("create");

    execute(rm_action(), &adapter).expect("a stopped sandbox is removed");

    assert_eq!(
        sandbox_store::load(&sandboxes_root(&home), "dev-1")
            .expect_err("the record is gone")
            .code(),
        "SANDBOX_NOT_FOUND"
    );
}

#[test]
fn a_broken_list_entry_can_still_be_removed() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new()
        .reply("create", CREATED, 0)
        .reply("status", UNREADABLE, 0)
        .reply("rm", REMOVED, 0);
    execute(create_action(), &adapter).expect("create");

    let listed = execute(SandboxAction::List, &adapter).expect("list");
    assert_eq!(
        listed["sandboxes"][0]["state"].as_str(),
        Some("broken"),
        "an unreadable status reports the record as broken"
    );

    execute(rm_action(), &adapter).expect("a broken record is removed through the provider");

    assert_eq!(
        sandbox_store::load(&sandboxes_root(&home), "dev-1")
            .expect_err("the record is gone")
            .code(),
        "SANDBOX_NOT_FOUND"
    );
}

#[test]
fn a_failed_rm_keeps_the_record_for_another_attempt() {
    let home = HomeGuard::new();
    let adapter = ScriptedAdapter::new()
        .reply("create", CREATED, 0)
        .reply("status", UNREADABLE, 0)
        .reply("rm", RM_FAILED, 0);
    execute(create_action(), &adapter).expect("create");

    let error = adapter_error(execute(rm_action(), &adapter).expect_err("the provider rm fails"));

    assert_eq!(error.code.as_str(), "ACTION_FAILED");
    assert_eq!(
        error.disposition,
        DeliverySemantics::unknown(),
        "a failed mutating provider call leaves delivery unknown"
    );
    assert!(
        sandbox_store::load(&sandboxes_root(&home), "dev-1").is_ok(),
        "the record stays so the next rm can retry"
    );
}
