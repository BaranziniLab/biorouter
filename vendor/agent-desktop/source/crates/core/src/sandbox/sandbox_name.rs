use crate::{AdapterError, AppError, ErrorCode};

const MAX_NAME_LEN: usize = 32;

pub(crate) fn validate_sandbox_name(name: &str) -> Result<(), AppError> {
    let bytes = name.as_bytes();
    let starts_well = bytes
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let rest_well = bytes
        .iter()
        .skip(1)
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if starts_well && rest_well && bytes.len() <= MAX_NAME_LEN {
        return Ok(());
    }
    Err(AppError::Adapter(
        AdapterError::new(
            ErrorCode::InvalidArgs,
            "Sandbox names use 1 to 32 lowercase letters, digits, or dashes, starting with a letter or digit.",
        )
        .with_suggestion("Pick a name such as `dev-1`, then run the command again."),
    ))
}

#[cfg(test)]
#[path = "sandbox_name_tests.rs"]
mod tests;
