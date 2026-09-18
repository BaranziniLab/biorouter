use anyhow::Result;
use std::env;
use std::path::PathBuf;
use tokio::process::Command;
use tokio::sync::OnceCell;

static SHELL_PATH_DIRS: OnceCell<Result<Vec<PathBuf>, anyhow::Error>> = OnceCell::const_new();

pub async fn get_shell_path_dirs() -> Result<&'static Vec<PathBuf>> {
    let result = SHELL_PATH_DIRS
        .get_or_init(|| async {
            get_shell_path_async()
                .await
                .map(|path| env::split_paths(&path).collect())
        })
        .await;

    match result {
        Ok(dirs) => Ok(dirs),
        Err(e) => Err(anyhow::anyhow!(
            "Failed to get shell PATH directories: {}",
            e
        )),
    }
}

async fn get_shell_path_async() -> Result<String> {
    #[cfg(not(windows))]
    let shell = {
        let s = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let allowed = [
            "/bin/bash",
            "/bin/zsh",
            "/bin/sh",
            "/usr/bin/bash",
            "/usr/bin/zsh",
            "/usr/bin/sh",
            "/usr/local/bin/bash",
            "/usr/local/bin/zsh",
        ];
        if allowed.contains(&s.as_str()) {
            s
        } else {
            tracing::warn!(
                "$SHELL '{}' is not in allowed list, falling back to /bin/bash",
                s
            );
            "/bin/bash".to_string()
        }
    };
    #[cfg(windows)]
    let shell = env::var("SHELL").unwrap_or_else(|_| "cmd".to_string());

    if cfg!(windows) {
        get_windows_path_async(&shell).await
    } else {
        get_unix_path_async(&shell).await
    }
    .or_else(|e| {
        tracing::warn!(
            "Failed to get PATH from shell ({}), falling back to current PATH",
            e
        );
        env::var("PATH").map_err(|_| anyhow::anyhow!("No PATH variable available"))
    })
}

async fn get_unix_path_async(shell: &str) -> Result<String> {
    let mut command = Command::new(shell);
    command.args(["-l", "-i", "-c", "echo $PATH"]);
    crate::developer::shell::strip_daemon_private_env(&mut command);
    crate::developer::shell::no_console_window(&mut command);
    let output = command
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute shell command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Shell command failed: {}", stderr));
    }

    let path = String::from_utf8(output.stdout)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in shell output: {}", e))?
        .trim()
        .to_string();

    if path.is_empty() {
        return Err(anyhow::anyhow!("Shell returned empty PATH"));
    }

    Ok(path)
}

async fn get_windows_path_async(shell: &str) -> Result<String> {
    let shell_name = std::path::Path::new(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cmd");

    let output = match shell_name {
        "pwsh" | "powershell" => {
            let mut command = Command::new(shell);
            command.args(["-NoLogo", "-Command", "$env:PATH"]);
            crate::developer::shell::strip_daemon_private_env(&mut command);
            crate::developer::shell::no_console_window(&mut command);
            command.output().await
        }
        _ => {
            let mut command = Command::new(shell);
            command.args(["/c", "echo %PATH%"]);
            crate::developer::shell::strip_daemon_private_env(&mut command);
            crate::developer::shell::no_console_window(&mut command);
            command.output().await
        }
    };

    let output = output.map_err(|e| anyhow::anyhow!("Failed to execute shell command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Shell command failed: {}", stderr));
    }

    let path = String::from_utf8(output.stdout)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in shell output: {}", e))?
        .trim()
        .to_string();

    if path.is_empty() {
        return Err(anyhow::anyhow!("Shell returned empty PATH"));
    }

    Ok(path)
}
