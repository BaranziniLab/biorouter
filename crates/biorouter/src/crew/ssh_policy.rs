//! Native configuration is trusted same-user/site-admin input. This preflight
//! checks effective policy; it is not a sandbox against config changes or Match exec.
use anyhow::{ensure, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

type Settings = HashMap<String, String>;

pub async fn preflight(args: &[String], target: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), inspect_route(args, target))
        .await
        .context("Crew SSH configuration preflight timed out")?
}

async fn inspect_route(args: &[String], target: &str) -> Result<()> {
    let config = args
        .windows(2)
        .find(|pair| pair[0] == "-F")
        .map(|pair| pair[1].clone());
    let mut invocation = args.to_vec();
    let mut host = target.to_string();
    let mut seen = HashSet::new();
    for depth in 0..16 {
        // Crew constructs separate option arguments, never bundled flags. Older
        // clients have only -f and do not report ForkAfterAuthentication in -G.
        ensure!(
            !invocation.iter().any(|arg| arg == "-f"),
            "Crew SSH cannot fork after authentication"
        );
        let settings = resolve(&invocation)
            .await
            .with_context(|| format!("Cannot inspect Crew SSH host {host}"))?;
        validate(&settings, &host, depth > 0)?;
        let Some(jump) = settings
            .get("proxyjump")
            .filter(|value| value.as_str() != "none")
        else {
            return Ok(());
        };
        ensure!(
            seen.insert((host.clone(), jump.clone())),
            "Crew SSH jump route contains a cycle at {host}"
        );
        if let Some(path) = &config {
            ensure!(
                shell_atom(path),
                "Crew SSH jump configuration path contains shell-sensitive characters; use a profile path containing only letters, digits, /, :, _, - and ."
            );
        }
        let (next, next_host) = jump_invocation(jump, &settings, config.as_deref())?;
        invocation = next;
        host = next_host;
    }
    anyhow::bail!("Crew SSH jump route exceeds sixteen hosts")
}

async fn resolve(args: &[String]) -> Result<Settings> {
    let mut command = tokio::process::Command::new("ssh");
    command
        .arg("-G")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::subprocess::prepare_agent_child_command(&mut command);
    let mut child = command.spawn()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .context("SSH configuration output unavailable")?
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() <= 1_048_576,
        "SSH configuration output exceeds one MiB"
    );
    ensure!(
        child.wait().await?.success(),
        "ssh -G refused the native configuration; inspect it with your SSH administrator"
    );
    let text = String::from_utf8(bytes).context("SSH configuration output is not UTF-8")?;
    let mut settings = Settings::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            // Only scalar fields are interpreted; arrays and paths are never reconstructed.
            settings
                .entry(key.to_string())
                .or_insert_with(|| value.trim().to_string());
        }
    }
    Ok(settings)
}

fn validate(settings: &Settings, host: &str, jump: bool) -> Result<()> {
    for (field, permitted, setting) in [
        (
            "stricthostkeychecking",
            &["yes", "true"][..],
            "StrictHostKeyChecking yes",
        ),
        ("forwardagent", &["no", "false"][..], "ForwardAgent no"),
        ("forwardx11", &["no", "false"][..], "ForwardX11 no"),
        (
            "permitlocalcommand",
            &["no", "false"][..],
            "PermitLocalCommand no",
        ),
        (
            "clearallforwardings",
            &["yes", "true"][..],
            "ClearAllForwardings yes",
        ),
        (
            "nohostauthenticationforlocalhost",
            &["no", "false"][..],
            "NoHostAuthenticationForLocalhost no",
        ),
        ("tunnel", &["no", "false"][..], "Tunnel no"),
    ] {
        ensure!(
            settings
                .get(field)
                .is_some_and(|value| permitted.contains(&value.as_str())),
            "Crew SSH host {host} requires {setting}; put this in its matching Host stanza before broader defaults"
        );
    }
    ensure!(
        settings
            .get("forkafterauthentication")
            .is_none_or(|value| matches!(value.as_str(), "no" | "false")),
        "Crew SSH host {host} requires ForkAfterAuthentication no; background authentication is not admitted"
    );
    ensure!(
        settings.get("proxycommand").is_none_or(|v| v == "none"),
        "Crew SSH host {host} uses a custom ProxyCommand; use native ProxyJump so every SSH hop can be checked"
    );
    ensure!(
        settings
            .get("gssapidelegatecredentials")
            .is_none_or(|value| matches!(value.as_str(), "no" | "false")),
        "Crew SSH host {host} requires GSSAPIDelegateCredentials no; delegated credentials are not admitted"
    );
    if jump {
        for (field, permitted, setting) in [
            ("controlmaster", &["no", "false"][..], "ControlMaster no"),
            (
                "controlpersist",
                &["no", "false", "0"][..],
                "ControlPersist no",
            ),
        ] {
            ensure!(
                settings
                    .get(field)
                    .is_some_and(|value| permitted.contains(&value.as_str())),
                "Crew SSH jump host {host} requires {setting}; inherited multiplexed masters are not admitted"
            );
        }
        ensure!(
            settings.get("controlpath").is_none_or(|v| v == "none"),
            "Crew SSH jump host {host} requires ControlPath none; an unrelated master could bypass host authentication"
        );
    }
    Ok(())
}

fn shell_atom(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/:_.-".contains(&b))
}

fn jump_invocation(
    route: &str,
    destination: &Settings,
    config: Option<&str>,
) -> Result<(Vec<String>, String)> {
    let (remaining, last) = route
        .rsplit_once(',')
        .map_or((None, route), |(rest, last)| (Some(rest), last));
    ensure!(
        route
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/@:_.-,[]".contains(&b)),
        "Crew SSH ProxyJump contains unsupported shell-sensitive syntax; use literal SSH host aliases"
    );
    let (user, host, port) = jump_authority(last)?;
    let mut args = Vec::new();
    if let Some(user) = user {
        args.extend(["-l".into(), user]);
    }
    if let Some(port) = port {
        args.extend(["-p".into(), port.to_string()]);
    }
    if let Some(rest) = remaining {
        args.extend(["-J".into(), rest.into()]);
    }
    if let Some(path) = config {
        args.extend(["-F".into(), path.into()]);
    }
    let hostname = destination
        .get("hostname")
        .context("SSH resolved hostname missing")?;
    ensure!(
        shell_atom(hostname) && !hostname.starts_with('-'),
        "Crew SSH resolved hostname contains unsupported shell-sensitive characters"
    );
    let port: u16 = destination
        .get("port")
        .context("SSH resolved port missing")?
        .parse()?;
    ensure!(port > 0, "SSH resolved port must be positive");
    args.extend(["-W".into(), format!("[{hostname}]:{port}"), host.clone()]);
    Ok((args, host))
}

fn jump_authority(value: &str) -> Result<(Option<String>, String, Option<u16>)> {
    let authority = value.strip_prefix("ssh://").unwrap_or(value);
    let (user, address) = match authority.split_once('@') {
        Some((user, address)) => {
            ensure!(
                !user.is_empty()
                    && user
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
                "Crew SSH jump username contains unsupported syntax"
            );
            (Some(user.to_string()), address)
        }
        None => (None, authority),
    };
    let (host, port) = if let Some(bracketed) = address.strip_prefix('[') {
        let (host, suffix) = bracketed
            .split_once(']')
            .context("Unclosed SSH jump host bracket")?;
        let port = if suffix.is_empty() {
            None
        } else {
            Some(
                suffix
                    .strip_prefix(':')
                    .context("Invalid SSH jump host suffix")?,
            )
        };
        (host, port)
    } else {
        address
            .split_once(':')
            .map_or((address, None), |(host, port)| (host, Some(port)))
    };
    // OpenSSH's diagnostic dump brackets numeric IPv4 as well as IPv6.
    // Preserve aliases verbatim rather than applying URL host normalization.
    ensure!(
        !host.is_empty()
            && !host.starts_with('-')
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b":_.-".contains(&b)),
        "Crew SSH jump host contains unsupported syntax"
    );
    let port = port
        .map(|port| -> Result<u16> {
            ensure!(
                !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
                "Invalid SSH jump port"
            );
            let port: u16 = port.parse()?;
            ensure!(port > 0, "SSH jump port must be positive");
            Ok(port)
        })
        .transpose()?;
    Ok((user, host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn shell_config_path(path: &std::path::Path) -> String {
        let path = path.display().to_string();
        #[cfg(windows)]
        {
            path.replace('\\', "/")
        }
        #[cfg(not(windows))]
        {
            path
        }
    }

    fn policy_config(root: &std::path::Path, weak_gate: bool, cycle: bool) -> PathBuf {
        let config = root.join("ssh_config");
        let gate_jump = if cycle { "gate-b" } else { "none" };
        let gate_b_jump = if cycle { "gate-a" } else { "none" };
        let gate_a_forward = if weak_gate { "yes" } else { "no" };
        fs::write(
            &config,
            format!(
                "Host target\n  HostName 127.0.0.1\n  Port 2224\n  ProxyJump gate-a,gate-b\nHost gate-a\n  HostName 127.0.0.1\n  Port 2222\n  ForwardAgent {1}\n  ProxyJump {2}\nHost gate-b\n  HostName 127.0.0.1\n  Port 2223\n  ProxyJump {3}\nHost *\n  User test\n  BatchMode yes\n  IdentitiesOnly yes\n  IdentityAgent none\n  StrictHostKeyChecking yes\n  UserKnownHostsFile {0}/known_hosts\n  ForwardAgent no\n  ForwardX11 no\n  PermitLocalCommand no\n  ClearAllForwardings yes\n  NoHostAuthenticationForLocalhost no\n  Tunnel no\n  ForkAfterAuthentication no\n  GSSAPIDelegateCredentials no\n  ControlMaster no\n  ControlPersist no\n  ControlPath none\n",
                shell_config_path(root), gate_a_forward, gate_jump, gate_b_jump
            ),
        )
        .unwrap();
        config
    }

    #[test]
    fn jump_invocation_preserves_remaining_jump_and_uses_resolved_destination() {
        let mut destination = Settings::new();
        destination.insert("hostname".into(), "127.0.0.1".into());
        destination.insert("port".into(), "2223".into());
        let (args, host) = jump_invocation(
            "gate-a,gate-b",
            &destination,
            Some("/tmp/profile-ssh_config"),
        )
        .unwrap();
        assert_eq!(
            args,
            vec![
                "-J",
                "gate-a",
                "-F",
                "/tmp/profile-ssh_config",
                "-W",
                "[127.0.0.1]:2223",
                "gate-b"
            ]
        );
        assert_eq!(host, "gate-b");
    }

    #[test]
    fn jump_invocation_rejects_shell_sensitive_route_and_destination() {
        let mut destination = Settings::new();
        destination.insert("hostname".into(), "127.0.0.1".into());
        destination.insert("port".into(), "2223".into());
        let err = jump_invocation("gate-a;touch /tmp/pwned", &destination, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported shell-sensitive"), "{err}");
        destination.insert("hostname".into(), "$(touch /tmp/pwned)".into());
        let err = jump_invocation("gate-b", &destination, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported shell-sensitive"), "{err}");
    }

    #[test]
    fn jump_authority_preserves_open_ssh_dump_hosts_and_ports() {
        assert_eq!(
            jump_authority("alice@[127.0.0.1]:2201").unwrap(),
            (Some("alice".into()), "127.0.0.1".into(), Some(2201))
        );
        assert_eq!(
            jump_authority("ssh://bob@[2001:0db8:0:0:0:0:0:1]:2202").unwrap(),
            (
                Some("bob".into()),
                "2001:0db8:0:0:0:0:0:1".into(),
                Some(2202)
            )
        );
        assert_eq!(
            jump_authority("CasePreserved-Alias").unwrap(),
            (None, "CasePreserved-Alias".into(), None)
        );
        let mut destination = Settings::new();
        destination.insert("hostname".into(), "target.example".into());
        destination.insert("port".into(), "22".into());
        let (args, _) =
            jump_invocation("alice@[2001:0db8:0:0:0:0:0:1]:2202", &destination, None).unwrap();
        assert_eq!(
            args,
            vec![
                "-l",
                "alice",
                "-p",
                "2202",
                "-W",
                "[target.example]:22",
                "2001:0db8:0:0:0:0:0:1"
            ]
        );
    }

    fn safe_validation_settings() -> Settings {
        [
            ("stricthostkeychecking", "yes"),
            ("forwardagent", "no"),
            ("forwardx11", "no"),
            ("permitlocalcommand", "no"),
            ("clearallforwardings", "yes"),
            ("nohostauthenticationforlocalhost", "no"),
            ("tunnel", "no"),
            ("proxycommand", "none"),
            ("gssapidelegatecredentials", "no"),
            ("controlmaster", "no"),
            ("controlpersist", "no"),
            ("controlpath", "none"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    #[test]
    fn validation_accepts_legacy_dump_without_optional_fork_field() {
        let settings = safe_validation_settings();
        validate(&settings, "legacy-gate", true).unwrap();
    }

    #[test]
    fn validation_rejects_reported_fork_after_authentication_yes() {
        let mut settings = safe_validation_settings();
        settings.insert("forkafterauthentication".into(), "yes".into());
        let error = validate(&settings, "unsafe-gate", true).unwrap_err();
        assert!(error.to_string().contains("ForkAfterAuthentication no"));
    }

    #[test]
    fn shell_atom_rejects_raw_backslashes_and_spaces() {
        assert!(!shell_atom(r"C:\Users\runner\profile"));
        assert!(!shell_atom("/tmp/profile with space"));
    }

    #[tokio::test]
    async fn native_preflight_accepts_safe_two_hop_config() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("known_hosts"), "").unwrap();
        let config = policy_config(root.path(), false, false);
        preflight(
            &["-F".into(), shell_config_path(&config), "target".into()],
            "target",
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn native_preflight_accepts_direct_route_without_optional_fork_field() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("known_hosts"), "").unwrap();
        let config = policy_config(root.path(), false, false);
        let mut text = fs::read_to_string(&config).unwrap();
        text = text.replace("  ProxyJump gate-a,gate-b\n", "");
        text = text.replace("  ForkAfterAuthentication no\n", "");
        fs::write(&config, text).unwrap();
        preflight(
            &["-F".into(), shell_config_path(&config), "target".into()],
            "target",
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn native_preflight_rejects_weak_implicit_jump_before_connecting() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("known_hosts"), "").unwrap();
        let config = policy_config(root.path(), true, false);
        let err = preflight(
            &["-F".into(), shell_config_path(&config), "target".into()],
            "target",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("ForwardAgent no"), "{err}");
    }

    #[tokio::test]
    async fn native_preflight_rejects_jump_cycle() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("known_hosts"), "").unwrap();
        let config = policy_config(root.path(), false, true);
        let err = preflight(
            &["-F".into(), shell_config_path(&config), "target".into()],
            "target",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("cycle") || err.contains("exceeds sixteen"),
            "{err}"
        );
    }
}
