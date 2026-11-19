use anyhow::Result;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::agents::types::SessionConfig;
use crate::agents::types::{
    RetryConfig, SuccessCheck, DEFAULT_ON_FAILURE_TIMEOUT_SECONDS, DEFAULT_RETRY_TIMEOUT_SECONDS,
};
use crate::config::Config;
use crate::conversation::message::Message;
use crate::conversation::Conversation;

/// Result of a retry logic evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum RetryResult {
    /// No retry configuration or session available, retry logic skipped
    Skipped,
    /// Maximum retry attempts reached, cannot retry further
    MaxAttemptsReached,
    /// Success checks passed, no retry needed
    SuccessChecksPassed,
    /// Retry is needed and will be performed
    Retried,
}

/// Environment variable for configuring retry timeout globally
const BIOROUTER_WORKFLOW_RETRY_TIMEOUT_SECONDS: &str = "BIOROUTER_WORKFLOW_RETRY_TIMEOUT_SECONDS";

/// Environment variable for configuring on_failure timeout globally
const BIOROUTER_WORKFLOW_ON_FAILURE_TIMEOUT_SECONDS: &str =
    "BIOROUTER_WORKFLOW_ON_FAILURE_TIMEOUT_SECONDS";

/// Manages retry state and operations for agent execution
#[derive(Debug)]
pub struct RetryManager {
    /// Current number of retry attempts
    attempts: Arc<Mutex<u32>>,
    last_result: Mutex<std::result::Result<RetryResult, String>>,
}

impl Default for RetryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RetryManager {
    /// Create a new retry manager
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(0)),
            last_result: Mutex::new(Ok(RetryResult::Skipped)),
        }
    }

    /// Reset the retry attempts counter to 0
    pub async fn reset_attempts(&self) {
        *self.attempts.lock().await = 0;
        *self.last_result.lock().await = Ok(RetryResult::Skipped);
    }

    /// Increment the retry attempts counter and return the new value
    pub async fn increment_attempts(&self) -> u32 {
        let mut attempts = self.attempts.lock().await;
        *attempts += 1;
        *attempts
    }

    /// Get the current retry attempts count
    pub async fn get_attempts(&self) -> u32 {
        *self.attempts.lock().await
    }

    pub(super) async fn last_result(&self) -> Result<RetryResult> {
        self.last_result
            .lock()
            .await
            .clone()
            .map_err(anyhow::Error::msg)
    }

    /// Reset status for retry: clear message history and final output tool state
    async fn reset_status_for_retry(
        messages: &mut Conversation,
        initial_messages: &[Message],
        final_output_tool: &Arc<Mutex<Option<crate::agents::final_output_tool::FinalOutputTool>>>,
    ) {
        *messages = Conversation::new_unvalidated(initial_messages.to_vec());
        info!("Reset message history to initial state for retry");

        if let Some(final_output_tool) = final_output_tool.lock().await.as_mut() {
            final_output_tool.final_output = None;
            info!("Cleared final output tool state for retry");
        }
    }

    pub async fn handle_retry_logic(
        &self,
        messages: &mut Conversation,
        session_config: &SessionConfig,
        initial_messages: &[Message],
        final_output_tool: &Arc<Mutex<Option<crate::agents::final_output_tool::FinalOutputTool>>>,
    ) -> Result<RetryResult> {
        let result = self
            .evaluate_retry_logic(
                messages,
                session_config,
                initial_messages,
                final_output_tool,
            )
            .await;
        // Reply streams can end normally after exhaustion or a check error.
        *self.last_result.lock().await = match &result {
            Ok(result) => Ok(result.clone()),
            Err(error) => Err(error.to_string()),
        };
        result
    }

    async fn evaluate_retry_logic(
        &self,
        messages: &mut Conversation,
        session_config: &SessionConfig,
        initial_messages: &[Message],
        final_output_tool: &Arc<Mutex<Option<crate::agents::final_output_tool::FinalOutputTool>>>,
    ) -> Result<RetryResult> {
        let Some(retry_config) = &session_config.retry_config else {
            return Ok(RetryResult::Skipped);
        };

        let success = execute_success_checks(&retry_config.checks, retry_config).await?;

        if success {
            info!("All success checks passed, no retry needed");
            return Ok(RetryResult::SuccessChecksPassed);
        }

        let current_attempts = self.get_attempts().await;
        if current_attempts >= retry_config.max_retries {
            let error_msg = Message::assistant().with_text(format!(
                "Maximum retry attempts ({}) exceeded. Unable to complete the task successfully.",
                retry_config.max_retries
            ));
            messages.push(error_msg);
            warn!(
                "Maximum retry attempts ({}) exceeded",
                retry_config.max_retries
            );
            return Ok(RetryResult::MaxAttemptsReached);
        }

        if let Some(on_failure_cmd) = &retry_config.on_failure {
            info!("Executing on_failure command: {}", on_failure_cmd);
            execute_on_failure_command(on_failure_cmd, retry_config).await?;
        }

        Self::reset_status_for_retry(messages, initial_messages, final_output_tool).await;

        let new_attempts = self.increment_attempts().await;
        info!("Incrementing retry attempts to {}", new_attempts);

        Ok(RetryResult::Retried)
    }
}

/// Get the configured timeout duration for retry operations
/// retry_config.timeout_seconds -> env var -> default
fn get_retry_timeout(retry_config: &RetryConfig) -> Duration {
    let timeout_seconds = retry_config
        .timeout_seconds
        .or_else(|| {
            let config = Config::global();
            config
                .get_param(BIOROUTER_WORKFLOW_RETRY_TIMEOUT_SECONDS)
                .ok()
        })
        .unwrap_or(DEFAULT_RETRY_TIMEOUT_SECONDS);

    Duration::from_secs(timeout_seconds)
}

/// Get the configured timeout duration for on_failure operations
/// retry_config.on_failure_timeout_seconds -> env var -> default
fn get_on_failure_timeout(retry_config: &RetryConfig) -> Duration {
    let timeout_seconds = retry_config
        .on_failure_timeout_seconds
        .or_else(|| {
            let config = Config::global();
            config
                .get_param(BIOROUTER_WORKFLOW_ON_FAILURE_TIMEOUT_SECONDS)
                .ok()
        })
        .unwrap_or(DEFAULT_ON_FAILURE_TIMEOUT_SECONDS);

    Duration::from_secs(timeout_seconds)
}

/// Outcome of a single [`SuccessCheck`].
///
/// Shared by the workflow retry path ([`execute_success_checks`]) and the
/// interactive done-ness gate ([`collect_check_failures`], BR-48), so both
/// evaluate a check with identical semantics — the gate just surfaces the
/// `Fail` reason to the model instead of resetting progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The check held.
    Pass,
    /// The check did not hold; the string is a short, human-readable reason.
    Fail(String),
}

/// Cap on how much command output is echoed into a failure reason, so a single
/// failing check cannot inject a huge blob back into the conversation.
const CHECK_OUTPUT_TAIL_CHARS: usize = 600;

/// The trailing `CHECK_OUTPUT_TAIL_CHARS` characters of some command output,
/// trimmed, for a compact failure reason.
fn output_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= CHECK_OUTPUT_TAIL_CHARS {
        trimmed.to_string()
    } else {
        let tail: String = trimmed
            .chars()
            .skip(count - CHECK_OUTPUT_TAIL_CHARS)
            .collect();
        format!("…{tail}")
    }
}

/// Resolve a possibly-relative check path against `cwd` when one is given.
fn resolve_check_path(path: &str, cwd: Option<&std::path::Path>) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    match cwd {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p.to_path_buf(),
    }
}

/// Run one [`SuccessCheck`], returning whether it held. `cwd`, when set, is the
/// working directory shell commands run in and the base for relative paths (the
/// session working dir for the interactive gate; `None` keeps the workflow
/// path's historical process-cwd behavior).
///
/// Returns `Err` only when a check could not be evaluated at all (e.g. a shell
/// command timed out) — callers decide whether that is fatal (workflow) or just
/// another surfaced failure (gate).
pub async fn run_success_check(
    check: &SuccessCheck,
    timeout: Duration,
    cwd: Option<&std::path::Path>,
) -> Result<CheckOutcome> {
    match check {
        SuccessCheck::Shell { command } => {
            let result = execute_shell_command_in(command, timeout, cwd).await?;
            if result.status.success() {
                Ok(CheckOutcome::Pass)
            } else {
                let stderr = output_tail(&result.stderr);
                let detail = if stderr.is_empty() {
                    output_tail(&result.stdout)
                } else {
                    stderr
                };
                Ok(CheckOutcome::Fail(format!(
                    "command `{command}` exited with status {}{}",
                    result.status,
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                )))
            }
        }
        SuccessCheck::FileExists { path } => {
            let resolved = resolve_check_path(path, cwd);
            if resolved.exists() {
                Ok(CheckOutcome::Pass)
            } else {
                Ok(CheckOutcome::Fail(format!(
                    "expected file `{path}` does not exist"
                )))
            }
        }
        SuccessCheck::OutputContains { command, substring } => {
            let result = execute_shell_command_in(command, timeout, cwd).await?;
            let mut combined = String::from_utf8_lossy(&result.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&result.stderr));
            if combined.contains(substring.as_str()) {
                Ok(CheckOutcome::Pass)
            } else {
                Ok(CheckOutcome::Fail(format!(
                    "output of `{command}` did not contain `{substring}`"
                )))
            }
        }
        SuccessCheck::JsonSchema { path, schema } => {
            let resolved = resolve_check_path(path, cwd);
            let contents = match tokio::fs::read_to_string(&resolved).await {
                Ok(contents) => contents,
                Err(e) => {
                    return Ok(CheckOutcome::Fail(format!("could not read `{path}`: {e}")));
                }
            };
            let value: serde_json::Value = match serde_json::from_str(&contents) {
                Ok(value) => value,
                Err(e) => {
                    return Ok(CheckOutcome::Fail(format!(
                        "`{path}` is not valid JSON: {e}"
                    )));
                }
            };
            let validator = match jsonschema::validator_for(schema) {
                Ok(validator) => validator,
                Err(e) => {
                    // A malformed schema is a configuration error, not the
                    // agent's fault — surface it so it can be fixed, don't crash.
                    return Ok(CheckOutcome::Fail(format!(
                        "invalid JSON Schema for `{path}` check: {e}"
                    )));
                }
            };
            let errors: Vec<String> = validator
                .iter_errors(&value)
                .map(|error| format!("{}: {error}", error.instance_path))
                .collect();
            if errors.is_empty() {
                Ok(CheckOutcome::Pass)
            } else {
                Ok(CheckOutcome::Fail(format!(
                    "`{path}` does not match the required JSON Schema: {}",
                    errors.join("; ")
                )))
            }
        }
    }
}

/// Execute all success checks and return true if all pass.
///
/// Short-circuits on the first failure and runs in the process working
/// directory, preserving the workflow retry path's behavior. New non-shell
/// variants (BR-48) are handled via [`run_success_check`], so workflows get
/// them for free.
pub async fn execute_success_checks(
    checks: &[SuccessCheck],
    retry_config: &RetryConfig,
) -> Result<bool> {
    let timeout = get_retry_timeout(retry_config);

    for check in checks {
        match run_success_check(check, timeout, None).await? {
            CheckOutcome::Pass => {
                info!("Success check passed: {check:?}");
            }
            CheckOutcome::Fail(reason) => {
                warn!("Success check failed: {reason}");
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Run every check and collect the reasons of those that failed (empty = all
/// passed). Unlike [`execute_success_checks`] this does NOT short-circuit — the
/// done-ness gate (BR-48) wants the full list so the model can fix everything at
/// once — and a check that could not be evaluated (timeout/IO error) becomes a
/// surfaced failure rather than aborting the gate. `cwd` is the session working
/// directory the checks run against.
pub async fn collect_check_failures(
    checks: &[SuccessCheck],
    timeout: Duration,
    cwd: Option<&std::path::Path>,
) -> Vec<String> {
    let mut failures = Vec::new();
    for check in checks {
        match run_success_check(check, timeout, cwd).await {
            Ok(CheckOutcome::Pass) => {}
            Ok(CheckOutcome::Fail(reason)) => failures.push(reason),
            Err(e) => failures.push(format!("check could not run: {e}")),
        }
    }
    failures
}

/// Execute a shell command with cross-platform compatibility and mandatory
/// timeout, in the process working directory.
pub async fn execute_shell_command(
    command: &str,
    timeout: std::time::Duration,
) -> Result<std::process::Output> {
    execute_shell_command_in(command, timeout, None).await
}

/// Like [`execute_shell_command`] but runs `command` in `cwd` when one is given
/// (BR-48: the interactive done-ness gate runs its checks in the session
/// working directory; the workflow path passes `None` for the historical
/// process-cwd behavior).
pub async fn execute_shell_command_in(
    command: &str,
    timeout: std::time::Duration,
    cwd: Option<&std::path::Path>,
) -> Result<std::process::Output> {
    debug!(
        "Executing shell command with timeout {:?} (cwd={:?}): {}",
        timeout, cwd, command
    );

    let future = async {
        #[cfg(target_os = "windows")]
        let mut cmd = {
            use std::os::windows::process::CommandExt;

            let mut cmd = Command::new("cmd.exe");
            cmd.args(["/D", "/S", "/C"]);
            // cmd.exe parses its /C payload itself, so preserve its shell quotes verbatim.
            cmd.as_std_mut().raw_arg(format!("\"{command}\""));
            cmd
        };
        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", command]);
            cmd
        };
        cmd.env("BIOROUTER_TERMINAL", "1");

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        crate::subprocess::prepare_agent_child_command(&mut cmd);
        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await?;

        debug!(
            "Shell command completed with status: {}, stdout: {}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        Ok(output)
    };

    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => {
            let error_msg = format!("Shell command timed out after {:?}: {}", timeout, command);
            warn!("{}", error_msg);
            Err(anyhow::anyhow!("{}", error_msg))
        }
    }
}

/// Execute an on_failure command and return an error if it fails
pub async fn execute_on_failure_command(command: &str, retry_config: &RetryConfig) -> Result<()> {
    let timeout = get_on_failure_timeout(retry_config);
    info!(
        "Executing on_failure command with timeout {:?}: {}",
        timeout, command
    );

    let output = match execute_shell_command(command, timeout).await {
        Ok(output) => output,
        Err(e) => {
            if e.to_string().contains("timed out") {
                let error_msg = format!(
                    "On_failure command timed out after {:?}: {}",
                    timeout, command
                );
                warn!("{}", error_msg);
                return Err(anyhow::anyhow!(error_msg));
            } else {
                warn!("On_failure command execution error: {}", e);
                return Err(e);
            }
        }
    };

    if !output.status.success() {
        let error_msg = format!(
            "On_failure command failed: command '{}' exited with status {}, stderr: {}",
            command,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        warn!("{}", error_msg);
        return Err(anyhow::anyhow!(error_msg));
    } else {
        info!("On_failure command completed successfully: {}", command);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::SuccessCheck;

    fn create_test_retry_config() -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            checks: vec![],
            on_failure: None,
            timeout_seconds: Some(60),
            on_failure_timeout_seconds: Some(120),
        }
    }

    async fn evaluate_observed_retry(
        manager: &RetryManager,
        retry_config: Option<RetryConfig>,
    ) -> Result<RetryResult> {
        let config = SessionConfig {
            id: "retry-observation".to_string(),
            schedule_id: None,
            max_turns: None,
            max_tool_calls: None,
            budget: None,
            retry_config,
            reasoning_effort: None,
        };
        manager
            .handle_retry_logic(
                &mut Conversation::new_unvalidated(Vec::new()),
                &config,
                &[],
                &Arc::new(Mutex::new(None)),
            )
            .await
    }

    #[tokio::test]
    async fn wave3_retry_observation_resets_between_replies() {
        let manager = RetryManager::new();
        let dir = tempfile::tempdir().unwrap();
        let required = dir.path().join("required");
        let config = RetryConfig {
            max_retries: 1,
            checks: vec![SuccessCheck::FileExists {
                path: required.to_string_lossy().into_owned(),
            }],
            ..create_test_retry_config()
        };
        assert_eq!(
            evaluate_observed_retry(&manager, Some(config.clone()))
                .await
                .unwrap(),
            RetryResult::Retried
        );
        assert_eq!(manager.get_attempts().await, 1);
        assert_eq!(
            evaluate_observed_retry(&manager, Some(config.clone()))
                .await
                .unwrap(),
            RetryResult::MaxAttemptsReached
        );
        assert_eq!(
            manager.last_result().await.unwrap(),
            RetryResult::MaxAttemptsReached
        );

        manager.reset_attempts().await;
        assert_eq!(manager.get_attempts().await, 0);
        assert_eq!(manager.last_result().await.unwrap(), RetryResult::Skipped);
        evaluate_observed_retry(&manager, None).await.unwrap();
        assert_eq!(manager.last_result().await.unwrap(), RetryResult::Skipped);

        std::fs::write(required, "complete").unwrap();
        evaluate_observed_retry(&manager, Some(config))
            .await
            .unwrap();
        assert_eq!(
            manager.last_result().await.unwrap(),
            RetryResult::SuccessChecksPassed
        );
    }

    #[tokio::test]
    async fn wave3_retry_observation_preserves_check_and_on_failure_errors() {
        let manager = RetryManager::new();
        let dir = tempfile::tempdir().unwrap();
        let slow_command = if cfg!(target_os = "windows") {
            "ping -n 3 127.0.0.1 >NUL"
        } else {
            "sleep 1"
        };
        for (checks, on_failure, timeout_seconds, expected) in [
            (
                vec![SuccessCheck::Shell {
                    command: slow_command.to_string(),
                }],
                None,
                Some(0),
                "timed out",
            ),
            (
                vec![SuccessCheck::FileExists {
                    path: dir.path().join("missing").to_string_lossy().into_owned(),
                }],
                Some("exit 7".to_string()),
                Some(10),
                "On_failure command failed",
            ),
        ] {
            manager.reset_attempts().await;
            let error = evaluate_observed_retry(
                &manager,
                Some(RetryConfig {
                    checks,
                    on_failure,
                    timeout_seconds,
                    ..create_test_retry_config()
                }),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
            assert_eq!(
                manager.last_result().await.unwrap_err().to_string(),
                error.to_string()
            );
            manager.reset_attempts().await;
            assert_eq!(manager.last_result().await.unwrap(), RetryResult::Skipped);
        }
    }

    #[test]
    fn test_retry_result_enum() {
        assert_ne!(RetryResult::Skipped, RetryResult::MaxAttemptsReached);
        assert_ne!(RetryResult::Skipped, RetryResult::SuccessChecksPassed);
        assert_ne!(RetryResult::Skipped, RetryResult::Retried);
        assert_ne!(
            RetryResult::MaxAttemptsReached,
            RetryResult::SuccessChecksPassed
        );
        assert_ne!(RetryResult::MaxAttemptsReached, RetryResult::Retried);
        assert_ne!(RetryResult::SuccessChecksPassed, RetryResult::Retried);

        let result = RetryResult::Retried;
        let cloned = result.clone();
        assert_eq!(result, cloned);

        let debug_str = format!("{:?}", RetryResult::MaxAttemptsReached);
        assert!(debug_str.contains("MaxAttemptsReached"));
    }

    #[tokio::test]
    async fn test_execute_success_checks_all_pass() {
        let checks = vec![
            SuccessCheck::Shell {
                command: "echo 'test'".to_string(),
            },
            SuccessCheck::Shell {
                command: "true".to_string(),
            },
        ];
        let retry_config = create_test_retry_config();

        let result = execute_success_checks(&checks, &retry_config).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_execute_success_checks_one_fails() {
        let checks = vec![
            SuccessCheck::Shell {
                command: "echo 'test'".to_string(),
            },
            SuccessCheck::Shell {
                command: "false".to_string(),
            },
        ];
        let retry_config = create_test_retry_config();

        let result = execute_success_checks(&checks, &retry_config).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_execute_shell_command_success() {
        let result = execute_shell_command("echo 'hello world'", Duration::from_secs(30)).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello world"));
    }

    #[tokio::test]
    async fn test_execute_shell_command_failure() {
        let result = execute_shell_command("false", Duration::from_secs(30)).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.status.success());
    }

    #[tokio::test]
    async fn test_execute_on_failure_command_success() {
        let retry_config = create_test_retry_config();
        let result = execute_on_failure_command("echo 'cleanup'", &retry_config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_on_failure_command_failure() {
        let retry_config = create_test_retry_config();
        let result = execute_on_failure_command("false", &retry_config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_shell_command_timeout() {
        let timeout = std::time::Duration::from_millis(100);
        let result = if cfg!(target_os = "windows") {
            execute_shell_command("ping -n 3 127.0.0.1 >NUL", timeout).await
        } else {
            execute_shell_command("sleep 1", timeout).await
        };

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_retry_timeout_uses_config_default() {
        let retry_config = RetryConfig {
            max_retries: 1,
            checks: vec![],
            on_failure: None,
            timeout_seconds: None,
            on_failure_timeout_seconds: None,
        };

        let timeout = get_retry_timeout(&retry_config);
        assert_eq!(timeout, Duration::from_secs(DEFAULT_RETRY_TIMEOUT_SECONDS));
    }

    #[tokio::test]
    async fn test_get_retry_timeout_uses_retry_config() {
        let retry_config = RetryConfig {
            max_retries: 1,
            checks: vec![],
            on_failure: None,
            timeout_seconds: Some(120),
            on_failure_timeout_seconds: None,
        };

        let timeout = get_retry_timeout(&retry_config);
        assert_eq!(timeout, Duration::from_secs(120));
    }

    #[tokio::test]
    async fn test_get_on_failure_timeout_uses_config_default() {
        let retry_config = RetryConfig {
            max_retries: 1,
            checks: vec![],
            on_failure: None,
            timeout_seconds: None,
            on_failure_timeout_seconds: None,
        };

        let timeout = get_on_failure_timeout(&retry_config);
        assert_eq!(
            timeout,
            Duration::from_secs(DEFAULT_ON_FAILURE_TIMEOUT_SECONDS)
        );
    }

    #[tokio::test]
    async fn test_get_on_failure_timeout_uses_retry_config() {
        let retry_config = RetryConfig {
            max_retries: 1,
            checks: vec![],
            on_failure: None,
            timeout_seconds: None,
            on_failure_timeout_seconds: Some(900),
        };

        let timeout = get_on_failure_timeout(&retry_config);
        assert_eq!(timeout, Duration::from_secs(900));
    }

    #[tokio::test]
    async fn test_on_failure_timeout_different_from_retry_timeout() {
        let retry_config = RetryConfig {
            max_retries: 1,
            checks: vec![],
            on_failure: None,
            timeout_seconds: Some(60),
            on_failure_timeout_seconds: Some(300),
        };

        let retry_timeout = get_retry_timeout(&retry_config);
        let on_failure_timeout = get_on_failure_timeout(&retry_config);

        assert_eq!(retry_timeout, Duration::from_secs(60));
        assert_eq!(on_failure_timeout, Duration::from_secs(300));
        assert_ne!(retry_timeout, on_failure_timeout);
    }

    // ---- BR-48: non-shell check variants + cwd resolution + collect-all ----

    const T: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn file_exists_check_passes_and_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("out.csv"), b"a,b\n1,2\n").unwrap();

        // Relative path resolves against the supplied cwd.
        let present = SuccessCheck::FileExists {
            path: "out.csv".to_string(),
        };
        assert_eq!(
            run_success_check(&present, T, Some(dir.path()))
                .await
                .unwrap(),
            CheckOutcome::Pass
        );

        let missing = SuccessCheck::FileExists {
            path: "nope.csv".to_string(),
        };
        match run_success_check(&missing, T, Some(dir.path()))
            .await
            .unwrap()
        {
            CheckOutcome::Fail(reason) => assert!(reason.contains("nope.csv")),
            CheckOutcome::Pass => panic!("missing file must fail"),
        }
    }

    #[tokio::test]
    async fn output_contains_check_ignores_exit_status() {
        // Passes on substring even though a plain Shell check of the same output
        // would only look at the (here successful) exit code.
        let pass = SuccessCheck::OutputContains {
            command: "echo '3 passed, 0 failed'".to_string(),
            substring: "0 failed".to_string(),
        };
        assert_eq!(
            run_success_check(&pass, T, None).await.unwrap(),
            CheckOutcome::Pass
        );

        let fail = SuccessCheck::OutputContains {
            command: "echo '2 passed, 1 failed'".to_string(),
            substring: "0 failed".to_string(),
        };
        match run_success_check(&fail, T, None).await.unwrap() {
            CheckOutcome::Fail(reason) => assert!(reason.contains("did not contain")),
            CheckOutcome::Pass => panic!("absent substring must fail"),
        }
    }

    #[tokio::test]
    async fn json_schema_check_validates_file_contents() {
        let dir = tempfile::TempDir::new().unwrap();
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string" } }
        });

        // Conforming file passes.
        std::fs::write(dir.path().join("ok.json"), br#"{"name":"x"}"#).unwrap();
        let ok = SuccessCheck::JsonSchema {
            path: "ok.json".to_string(),
            schema: schema.clone(),
        };
        assert_eq!(
            run_success_check(&ok, T, Some(dir.path())).await.unwrap(),
            CheckOutcome::Pass
        );

        // Schema violation fails with a path-scoped reason.
        std::fs::write(dir.path().join("bad.json"), br#"{"name":42}"#).unwrap();
        let bad = SuccessCheck::JsonSchema {
            path: "bad.json".to_string(),
            schema: schema.clone(),
        };
        match run_success_check(&bad, T, Some(dir.path())).await.unwrap() {
            CheckOutcome::Fail(reason) => assert!(reason.contains("JSON Schema")),
            CheckOutcome::Pass => panic!("schema mismatch must fail"),
        }

        // Missing file fails rather than erroring.
        let gone = SuccessCheck::JsonSchema {
            path: "gone.json".to_string(),
            schema,
        };
        match run_success_check(&gone, T, Some(dir.path())).await.unwrap() {
            CheckOutcome::Fail(reason) => assert!(reason.contains("could not read")),
            CheckOutcome::Pass => panic!("missing json file must fail"),
        }
    }

    #[tokio::test]
    async fn collect_check_failures_reports_all_not_just_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let checks = vec![
            SuccessCheck::Shell {
                command: "true".to_string(), // passes
            },
            SuccessCheck::Shell {
                command: "false".to_string(), // fails
            },
            SuccessCheck::FileExists {
                path: "missing.txt".to_string(), // fails
            },
        ];
        let failures = collect_check_failures(&checks, T, Some(dir.path())).await;
        // The passing check is omitted; both failures surface (no short-circuit).
        assert_eq!(failures.len(), 2, "all failures collected: {failures:?}");
        assert!(failures.iter().any(|f| f.contains("status")));
        assert!(failures.iter().any(|f| f.contains("missing.txt")));

        // All-pass yields an empty list (the gate's "done" signal).
        let all_pass = vec![SuccessCheck::Shell {
            command: "true".to_string(),
        }];
        assert!(collect_check_failures(&all_pass, T, Some(dir.path()))
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn shell_check_runs_in_supplied_cwd() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("marker"), b"").unwrap();
        // `ls marker` only succeeds when the command runs inside `dir`.
        let check = SuccessCheck::Shell {
            command: "ls marker".to_string(),
        };
        assert_eq!(
            run_success_check(&check, T, Some(dir.path()))
                .await
                .unwrap(),
            CheckOutcome::Pass
        );
    }
}
