use super::Connection;
use anyhow::{bail, ensure, Result};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
};

pub const MAX_FRAME: usize = 1_048_576;
pub struct Transport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    unusable: bool,
}

pub fn ssh_args(c: &Connection, control: &Path) -> Vec<String> {
    let mut args = vec![
        "-T".into(),
        "-S".into(),
        control.to_string_lossy().into_owned(),
    ];
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        let ssh = std::path::PathBuf::from(profile).join("home/.ssh");
        args.extend([
            "-F".into(),
            ssh.join("config").to_string_lossy().into_owned(),
            "-o".into(),
            format!("UserKnownHostsFile={}", ssh.join("known_hosts").display()),
            "-o".into(),
            "IdentityAgent=none".into(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
        ]);
    }
    for option in [
        "StrictHostKeyChecking=yes",
        "ForwardAgent=no",
        "ForwardX11=no",
        "PermitLocalCommand=no",
        "ClearAllForwardings=yes",
        "ConnectTimeout=30",
        "ServerAliveInterval=30",
        "ServerAliveCountMax=3",
    ] {
        args.extend(["-o".into(), option.into()]);
    }
    if let Some(port) = c.port {
        args.extend(["-p".into(), port.to_string()]);
    }
    if let Some(identity) = &c.identity_file {
        args.extend(["-i".into(), identity.clone()]);
    }
    if let Some(jump) = &c.proxy_jump {
        args.extend(["-J".into(), jump.clone()]);
    }
    args
}

impl Transport {
    pub async fn connect(c: &Connection, control: &Path) -> Result<Self> {
        ensure!(
            super::safe_atom(&c.ssh_target)
                && super::safe_atom(&c.socket_path)
                && c.socket_path.starts_with('/'),
            "Saved SSH connection contains invalid command arguments"
        );
        uuid::Uuid::parse_str(&c.workspace_id)?;
        let mut args = ssh_args(c, control);
        args.extend(["-o".into(), "BatchMode=yes".into(), "-o".into(), "ControlMaster=no".into(), "-o".into(), "ControlPersist=no".into(), c.ssh_target.clone(),
            // Every remote argument has a restricted grammar; no content or credential enters this shell command.
            format!("~/.local/bin/biorouter-crew bridge --stdio --socket {} --owner-uid {} --workspace-id {}", c.socket_path, c.owner_uid, c.workspace_id)]);
        super::ssh_policy::preflight(&args, &c.ssh_target).await?;
        let mut command = tokio::process::Command::new("ssh");
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        crate::subprocess::prepare_agent_child_command(&mut command);
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("SSH input unavailable"))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("SSH output unavailable"))?,
        );
        Ok(Self {
            child,
            stdin,
            stdout,
            unusable: false,
        })
    }
    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        auth: Option<Value>,
        credential: Option<&str>,
        id: Option<String>,
    ) -> Result<Value> {
        ensure!(!self.unusable, "Crew SSH transport is unusable; reconnect before issuing another request. Inspect any previously submitted operation before retrying");
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut frame = json!({"version":1,"id":id,"method":method,"params":params});
        if let Some(auth) = auth {
            frame["auth"] = auth;
        }
        if let Some(credential) = credential {
            frame["credential"] = json!(credential);
        }
        let mut bytes = serde_json::to_vec(&frame)?;
        ensure!(bytes.len() < MAX_FRAME, "Crew request exceeds frame limit");
        bytes.push(b'\n');
        // Cancellation after a write must never allow the next caller to consume
        // this request's late reply. Only a complete valid envelope rearms it.
        self.unusable = true;
        let mut stage = "write";
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            self.stdin.write_all(&bytes).await?;
            self.stdin.flush().await?;
            stage = "read";
            let mut response = Vec::new();
            let length = (&mut self.stdout)
                .take((MAX_FRAME + 1) as u64)
                .read_until(b'\n', &mut response)
                .await?;
            ensure!(
                length > 0,
                "SSH connection closed; authenticate or inspect host trust before reconnecting"
            );
            ensure!(
                length <= MAX_FRAME && response.last() == Some(&b'\n'),
                "Crew response exceeds frame limit or is incomplete"
            );
            stage = "frame validation";
            let response: Value = serde_json::from_slice(&response)?;
            ensure!(
                response.get("id").and_then(Value::as_str) == Some(id.as_str()),
                "Crew response ID mismatch"
            );
            if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
                ensure!(
                    error.get("code").is_some_and(Value::is_string)
                        && error.get("message").is_some_and(Value::is_string),
                    "Invalid broker error envelope"
                );
            } else {
                ensure!(
                    response.get("result").is_some(),
                    "Crew response has no result"
                );
            }
            Ok::<_, anyhow::Error>(response)
        })
        .await;
        match result {
            Ok(Ok(v)) => {
                self.unusable = false;
                if let Some(error) = v.get("error").filter(|v| !v.is_null()) {
                    bail!("Crew broker refused request: {}", error);
                }
                v.get("result")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Crew response has no result"))
            }
            Ok(Err(e)) => {
                let _ = self.child.kill().await;
                Err(e.context(format!("Crew SSH {stage} failed; reconnect. Submitted operation outcome may be unknown; inspect history before retrying")))
            }
            Err(_) => {
                let _ = self.child.kill().await;
                bail!("Crew request timed out; mutation outcome is unknown. Inspect history before retrying.")
            }
        }
    }
    pub fn is_usable(&self) -> bool {
        !self.unusable
    }
    pub async fn close(&mut self) {
        self.unusable = true;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

#[cfg(all(test, unix))]
#[path = "transport_tests.rs"]
mod tests;
