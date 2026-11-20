use super::execute;
use crate::refs::refs_test_support::HomeGuard;
use crate::sandbox::provider_test_support::ScriptedAdapter;

#[test]
fn an_unknown_provider_is_refused_before_any_script_runs() {
    let _home = HomeGuard::new();
    let adapter = ScriptedAdapter::new();

    let error = execute(Some("nomad".to_string()), &adapter).expect_err("unknown provider");

    assert_eq!(error.code(), "INVALID_ARGS");
    assert!(adapter.requests().is_empty());
}
