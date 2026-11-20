use crate::{AdapterError, AppError, ErrorCode};

const MAX_KEY_LEN: usize = 32;
const MAX_VALUE_BYTES: usize = 1024;

pub(crate) fn parse_settings(pairs: &[String]) -> Result<Vec<(String, String)>, AppError> {
    pairs.iter().map(|pair| parse_pair(pair)).collect()
}

fn parse_pair(pair: &str) -> Result<(String, String), AppError> {
    let (key, value) = pair
        .split_once('=')
        .ok_or_else(|| invalid(pair, "must look like key=value"))?;
    if !valid_key(key) {
        return Err(invalid(
            pair,
            "has a key that is not lowercase letters, digits, or underscores starting with a letter",
        ));
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(invalid(pair, "has a value longer than 1024 bytes"));
    }
    if value.contains(['\0', '\n', '\r']) {
        return Err(invalid(pair, "has a value with a NUL or a newline"));
    }
    Ok((
        format!("AD_SET_{}", key.to_ascii_uppercase()),
        value.to_string(),
    ))
}

fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.len() <= MAX_KEY_LEN
        && bytes.first().is_some_and(|b| b.is_ascii_lowercase())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

fn invalid(pair: &str, reason: &str) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::InvalidArgs,
            format!("Setting {pair:?} {reason}."),
        )
        .with_suggestion("Pass each setting as --set key=value with a lowercase key, for example --set ipsw=/path/to/image.ipsw."),
    )
}

#[cfg(test)]
#[path = "provider_settings_tests.rs"]
mod tests;
