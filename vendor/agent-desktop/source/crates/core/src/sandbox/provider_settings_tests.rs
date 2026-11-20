use super::parse_settings;

#[test]
fn rejects_an_uppercase_key_and_a_value_with_a_newline() {
    let key = parse_settings(&["Key=v".to_string()]).unwrap_err();
    assert_eq!(key.code(), "INVALID_ARGS");
    let value = parse_settings(&["key=a\nb".to_string()]).unwrap_err();
    assert_eq!(value.code(), "INVALID_ARGS");
}
