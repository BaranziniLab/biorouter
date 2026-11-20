use super::validate_sandbox_name;

#[test]
fn rejects_uppercase_and_underscores_with_invalid_args() {
    let error = validate_sandbox_name("Dev_1").unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGS");
}
