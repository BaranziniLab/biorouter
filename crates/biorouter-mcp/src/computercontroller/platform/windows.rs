use super::SystemAutomation;
use std::path::PathBuf;
use std::process::Command;

pub struct WindowsAutomation;

impl SystemAutomation for WindowsAutomation {
    fn execute_system_script(&self, script: &str) -> std::io::Result<String> {
        // `script` is written by the model, so this child is agent-controlled
        // and must not inherit the daemon's own credentials (issue #57).
        let mut cmd = Command::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .env("BIOROUTER_TERMINAL", "1");
        crate::developer::shell::strip_daemon_private_env_std(&mut cmd);
        crate::developer::shell::no_console_window_std(&mut cmd);
        let output = cmd.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

        // Honest reporting: surface non-zero exit codes and stderr instead of
        // silently returning empty stdout, so a failed automation script does
        // not look like a success and trigger blind retries.
        if output.status.success() {
            return Ok(stdout);
        }

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let mut msg = format!("powershell exited with status {code}");
        let trimmed_err = stderr.trim();
        if !trimmed_err.is_empty() {
            msg.push_str(&format!(": {trimmed_err}"));
        }
        let trimmed_out = stdout.trim();
        if !trimmed_out.is_empty() {
            msg.push_str(&format!(
                "\n\nPartial output before failure:\n{trimmed_out}"
            ));
        }
        Err(std::io::Error::other(msg))
    }

    fn get_shell_command(&self) -> (&'static str, &'static str) {
        ("powershell", "-Command")
    }

    fn get_temp_path(&self) -> PathBuf {
        std::env::var("TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(r"C:\Windows\Temp"))
    }
}
