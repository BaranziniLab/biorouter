use agent_desktop_core::{AdapterError, ErrorCode, ProviderScriptOutput, ProviderScriptRequest};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::Instant;

use super::run_with_deadline;

const PROVIDER_SCRIPT_INHERITED_ENV: [&str; 3] = ["PATH", "HOME", "TMPDIR"];
const TIMEOUT_MESSAGE: &str = "The provider script did not finish in time.";
const TIMEOUT_SUGGESTION: &str = "Run `agent-desktop sandbox status` to check the sandbox.";

pub(crate) fn run_provider_script(
    request: &ProviderScriptRequest,
) -> Result<ProviderScriptOutput, AdapterError> {
    let deadline = Instant::now().checked_add(request.timeout).ok_or_else(|| {
        AdapterError::timeout(TIMEOUT_MESSAGE).with_suggestion(TIMEOUT_SUGGESTION)
    })?;
    let mut command = Command::new("/bin/sh");
    command.arg(&request.script).args(&request.args).env_clear();
    for key in PROVIDER_SCRIPT_INHERITED_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.envs(request.env.iter().map(|(key, value)| (key, value)));
    command
        .current_dir(&request.working_dir)
        .stdin(Stdio::null());

    super::interrupt::forward_to_active_group();
    let output = run_with_deadline(&mut command, "Provider script", deadline)
        .map_err(|error| map_runner_error(error, request))?;
    let stderr_start = output.stderr.len().saturating_sub(4096);

    Ok(ProviderScriptOutput {
        status: output
            .status
            .code()
            .or_else(|| output.status.signal().map(|signal| 128 + signal)),
        stdout: output.stdout,
        stderr_tail: String::from_utf8_lossy(&output.stderr[stderr_start..]).into_owned(),
    })
}

fn map_runner_error(mut error: AdapterError, request: &ProviderScriptRequest) -> AdapterError {
    if error.code == ErrorCode::Timeout {
        return AdapterError::timeout(TIMEOUT_MESSAGE).with_suggestion(TIMEOUT_SUGGESTION);
    }
    if error
        .details
        .as_ref()
        .and_then(|details| details.get("kind"))
        .and_then(serde_json::Value::as_str)
        == Some("subprocess_output_limit")
    {
        return AdapterError::new(
            ErrorCode::ActionFailed,
            "The provider script printed more output than agent-desktop reads.",
        )
        .with_suggestion(format!(
            "Check the log files in the working directory {}.",
            request.working_dir.display()
        ));
    }

    let detail = error.message.clone();
    error.message = "The provider script could not be run.".to_owned();
    error.suggestion = Some(format!(
        "Check that the script at {} is readable and that {} is accessible.",
        request.script.display(),
        request.working_dir.display()
    ));
    if error.platform_detail.is_none() {
        error.platform_detail = Some(detail);
    }
    error
}

#[cfg(all(test, unix))]
#[path = "provider_script_tests.rs"]
mod tests;
