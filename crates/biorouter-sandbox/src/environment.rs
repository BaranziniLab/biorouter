//! Environment boundary shared by every process spawned on an agent's behalf.

use std::{env, ffi::OsString};

/// BioRouter's own environment namespaces. Everything outside them belongs to
/// the user or the OS and is passed through to children untouched.
const BIOROUTER_ENV_PREFIXES: &[&str] = &["BIOROUTER_"];

/// The daemon's private server configuration. Nothing in this namespace may
/// reach a process running agent-controlled code.
const DAEMON_PRIVATE_PREFIXES: &[&str] = &["BIOROUTER_SERVER__"];

/// Credential-shaped names applied only inside BioRouter's own namespaces.
const BIOROUTER_SECRET_MARKERS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "PASSPHRASE",
    "PASSCODE",
    "CREDENTIAL",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "SIGNING_KEY",
    "AUTH",
];

/// Whether `key` names a BioRouter-private variable that must not reach a
/// process spawned on an agent's behalf.
///
/// This is deny-by-default inside `BIOROUTER_SERVER__`, plus a
/// credential-shaped net over the rest of BioRouter's namespace. Variables
/// outside those namespaces, including extension credentials and the user's
/// normal environment, pass through unchanged.
pub fn is_daemon_private_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    if DAEMON_PRIVATE_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        return true;
    }
    BIOROUTER_ENV_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
        && BIOROUTER_SECRET_MARKERS
            .iter()
            .any(|marker| key.contains(marker))
}

/// Remove daemon-private variables from an asynchronous command, including
/// values explicitly set on the command.
pub fn strip_daemon_private_env(command: &mut tokio::process::Command) {
    let explicit = command
        .as_std()
        .get_envs()
        .filter(|(_, value)| value.is_some())
        .map(|(key, _)| key.to_os_string())
        .collect::<Vec<_>>();

    for key in doomed_env_keys(explicit) {
        command.env_remove(key);
    }
}

/// Remove daemon-private variables from a synchronous command, including
/// values explicitly set on the command.
pub fn strip_daemon_private_env_std(command: &mut std::process::Command) {
    let explicit = command
        .get_envs()
        .filter(|(_, value)| value.is_some())
        .map(|(key, _)| key.to_os_string())
        .collect::<Vec<_>>();

    for key in doomed_env_keys(explicit) {
        command.env_remove(key);
    }
}

fn doomed_env_keys(explicit: Vec<OsString>) -> Vec<OsString> {
    env::vars_os()
        .map(|(key, _)| key)
        .chain(explicit)
        .filter(|key| key.to_str().is_some_and(is_daemon_private_env_key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_biorouter_private_credentials() {
        let mut command = std::process::Command::new("printenv");
        command
            .env("BIOROUTER_SERVER__SECRET_KEY", "daemon-private")
            .env("BIOROUTER_ACP_WS_TOKEN", "daemon-private")
            .env("BIOROUTER_PORT", "4931")
            .env("SPOKEAGENT_PASSCODE", "extension-private")
            .env("PATH", "/usr/bin");

        strip_daemon_private_env_std(&mut command);

        let envs = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert!(envs.contains(&("BIOROUTER_SERVER__SECRET_KEY".to_string(), None)));
        assert!(envs.contains(&("BIOROUTER_ACP_WS_TOKEN".to_string(), None)));
        assert!(envs.contains(&("BIOROUTER_PORT".to_string(), Some("4931".to_string()))));
        assert!(envs.contains(&(
            "SPOKEAGENT_PASSCODE".to_string(),
            Some("extension-private".to_string())
        )));
        assert!(envs.contains(&("PATH".to_string(), Some("/usr/bin".to_string()))));
    }
}
