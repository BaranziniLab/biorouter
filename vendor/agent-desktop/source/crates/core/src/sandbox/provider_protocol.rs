use serde_json::Value;

use crate::sandbox::exec_passthrough::ExecPassthrough;
use crate::sandbox::provider_reply::ProviderReply;
use crate::sandbox::provider_script_output::ProviderScriptOutput;
use crate::sandbox::sandbox_state::SandboxState;
use crate::{AdapterError, AppError, DeliverySemantics, ErrorCode};

const MAX_PLATFORM_DETAIL_CHARS: usize = 2000;
const EXEC_PROVIDER_FAILURE_STATUS: i32 = 125;
const LIST_SUGGESTION: &str =
    "Run `agent-desktop sandbox list` to find the sandbox and run the command again.";

/// Reads the last stdout line as a contract reply, mapping a failure object to an error.
pub(crate) fn parse_reply(
    provider: &str,
    name: Option<&str>,
    mutating: bool,
    output: &ProviderScriptOutput,
) -> Result<ProviderReply, AppError> {
    let suggestion = next_step(name);
    let value = reply_value(&output.stdout)
        .ok_or_else(|| unreadable(provider, &suggestion, mutating, output))?;
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => success_reply(provider, &suggestion, mutating, &value, output),
        Some(false) => Err(failure_error(provider, &suggestion, mutating, &value, output).into()),
        None => Err(unreadable(provider, &suggestion, mutating, output).into()),
    }
}

/// Passes the guest envelope through unchanged unless the provider reports its own failure.
pub(crate) fn exec_outcome(
    provider: &str,
    output: &ProviderScriptOutput,
) -> Result<ExecPassthrough, AppError> {
    let Some(status) = output.status else {
        return Err(missing_status(provider, output).into());
    };
    if status == EXEC_PROVIDER_FAILURE_STATUS
        && let Some(error) = exec_failure(provider, output)
    {
        return Err(error.into());
    }
    Ok(ExecPassthrough {
        stdout: output.stdout.clone(),
        status,
    })
}

fn success_reply(
    provider: &str,
    suggestion: &str,
    mutating: bool,
    value: &Value,
    output: &ProviderScriptOutput,
) -> Result<ProviderReply, AppError> {
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .and_then(SandboxState::from_wire);
    let message = value.get("message").and_then(Value::as_str);
    let (Some(state), Some(message)) = (state, message) else {
        return Err(unreadable(provider, suggestion, mutating, output).into());
    };

    Ok(ProviderReply {
        state,
        message: message.to_owned(),
        next: value.get("next").and_then(Value::as_str).map(str::to_owned),
        data: value.get("data").cloned(),
    })
}

fn failure_error(
    provider: &str,
    suggestion: &str,
    mutating: bool,
    value: &Value,
    output: &ProviderScriptOutput,
) -> AdapterError {
    let Some((code, message, script_suggestion)) = failure_fields(value) else {
        return unreadable(provider, suggestion, mutating, output);
    };

    with_stderr(
        AdapterError::new(map_code(code), message)
            .with_suggestion(script_suggestion)
            .with_disposition(disposition(mutating, no_change_reported(value))),
        output,
    )
}

fn exec_failure(provider: &str, output: &ProviderScriptOutput) -> Option<AdapterError> {
    reply_value(&output.stdout)
        .filter(|value| value.get("ok").and_then(Value::as_bool) == Some(false))
        .filter(|value| failure_fields(value).is_some())
        .map(|value| failure_error(provider, LIST_SUGGESTION, true, &value, output))
}

fn missing_status(provider: &str, output: &ProviderScriptOutput) -> AdapterError {
    with_stderr(
        AdapterError::new(
            ErrorCode::ActionFailed,
            format!(
                "The {provider} provider stopped before agent-desktop could read its exit status."
            ),
        )
        .with_suggestion(LIST_SUGGESTION)
        .with_disposition(DeliverySemantics::unknown()),
        output,
    )
}

fn unreadable(
    provider: &str,
    suggestion: &str,
    mutating: bool,
    output: &ProviderScriptOutput,
) -> AdapterError {
    with_stderr(
        AdapterError::new(
            ErrorCode::ActionFailed,
            format!("The {provider} provider returned output agent-desktop could not read."),
        )
        .with_suggestion(suggestion)
        .with_disposition(disposition(mutating, false)),
        output,
    )
}

pub(crate) fn next_step(name: Option<&str>) -> String {
    match name {
        Some(name) => {
            format!("Run `agent-desktop sandbox status {name}` to see its current state.")
        }
        None => {
            "Run `agent-desktop sandbox install` to see each provider's current state.".to_string()
        }
    }
}

/// Whether the provider finished with a complete failure reply that does not say it changed anything.
pub(crate) fn provider_refused(output: &ProviderScriptOutput) -> bool {
    reply_value(&output.stdout).is_some_and(|value| {
        value.get("ok").and_then(Value::as_bool) == Some(false)
            && failure_fields(&value).is_some()
            && value.pointer("/data/changed").and_then(Value::as_bool) != Some(true)
    })
}

fn reply_value(stdout: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
    serde_json::from_str(line).ok()
}

fn failure_fields(value: &Value) -> Option<(&str, &str, &str)> {
    Some((
        value.get("code")?.as_str()?,
        value.get("message")?.as_str()?,
        value.get("suggestion")?.as_str()?,
    ))
}

fn map_code(code: &str) -> ErrorCode {
    match code {
        "INVALID_ARGS" => ErrorCode::InvalidArgs,
        "PLATFORM_NOT_SUPPORTED" => ErrorCode::PlatformNotSupported,
        "PERM_DENIED" => ErrorCode::PermDenied,
        "TIMEOUT" => ErrorCode::Timeout,
        "SANDBOX_NOT_FOUND" => ErrorCode::SandboxNotFound,
        _ => ErrorCode::ActionFailed,
    }
}

fn no_change_reported(value: &Value) -> bool {
    value
        .get("data")
        .and_then(|data| data.get("changed"))
        .and_then(Value::as_bool)
        == Some(false)
}

fn disposition(mutating: bool, no_change: bool) -> DeliverySemantics {
    if mutating && !no_change {
        DeliverySemantics::unknown()
    } else {
        DeliverySemantics::not_delivered()
    }
}

fn with_stderr(error: AdapterError, output: &ProviderScriptOutput) -> AdapterError {
    match platform_detail(output) {
        Some(detail) => error.with_platform_detail(detail),
        None => error,
    }
}

fn platform_detail(output: &ProviderScriptOutput) -> Option<String> {
    let tail = output.stderr_tail.trim();
    if tail.is_empty() {
        return None;
    }
    Some(tail.chars().take(MAX_PLATFORM_DETAIL_CHARS).collect())
}

#[cfg(test)]
#[path = "provider_protocol_tests.rs"]
mod tests;
