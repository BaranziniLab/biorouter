use tokio::process::Command;

/// Suppress a child's console window on Windows.
///
/// ⚠ **This is a re-export, not a second implementation.** The flag itself lives
/// in [`biorouter_sandbox::console`], because `biorouter` depends on
/// `biorouter-mcp` which depends on `biorouter-sandbox` — never the reverse — so
/// the hot spawn sites in `biorouter-mcp` (`developer__shell`, background jobs,
/// Computer Use, Agent Drafter) cannot reach a helper defined *here*
/// without a dependency cycle. That is exactly why they ran without the flag and
/// flashed a console window on every tool call. Keeping one definition in the
/// leaf crate is what makes a single fix cover every layer.
///
/// Reached through `biorouter-mcp`'s re-export rather than from
/// `biorouter-sandbox` directly, because this crate does not depend on the
/// sandbox crate — only on `biorouter-mcp`, which does.
pub use biorouter_mcp::developer::shell::no_console_window as configure_command_no_window;

/// Prepare a child process that runs on the agent's behalf.
///
/// Two things every such child needs, in one place so a new spawn site cannot
/// get only one of them:
///
/// 1. no console window on Windows ([`configure_command_no_window`]);
/// 2. none of the daemon's own credentials
///    ([`biorouter_mcp::developer::shell::strip_daemon_private_env`], issue #57).
///
/// The second is the security-relevant half. `biorouterd` holds
/// `BIOROUTER_SERVER__SECRET_KEY` in its environment, and a child that inherits
/// it becomes a fully authenticated client of the daemon's own REST API — it
/// can read or import *any* session, which defeats every control that lives in
/// the agent loop.
///
/// Callers today are hook commands (which run on agent activity), the llama.cpp
/// sidecar, the Azure auth helper, the retry probe, and the CLI-agent providers
/// `claude_code` and `codex`. It matters most for those last two: they spawn
/// another vendor's coding agent — itself an agent with a shell — and are
/// therefore the highest-privilege children the daemon creates, so an inherited
/// secret key there is the worst case this strip exists to prevent.
///
/// Those two providers apply a second, different scrub on top of this one.
/// [`crate::providers::coding_agent::env::configure_subscription_child`] removes
/// the *user's* inference credentials, so the run stays on the subscription the
/// user's CLI is signed in to instead of being silently rerouted onto a metered
/// API account, whereas this function removes the *daemon's* credentials and
/// deliberately keeps the user's. The two policies are complementary and both
/// are needed, which is why that function applies its own scrub and then calls
/// this one, so that the ordering rule below still holds for a coding-agent
/// child.
///
/// Call this **last**, after every other `env`/`envs` call on the command: the
/// strip and `env` write to the same map, so a later `env` would re-admit a
/// credential the strip had already removed.
pub fn prepare_agent_child_command(command: &mut Command) {
    configure_command_no_window(command);
    biorouter_mcp::developer::shell::strip_daemon_private_env(command);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The daemon's own credentials are removed, and nothing else is — the
    /// user's environment and an extension's declared credential are not ours
    /// to censor (issue #24 was a truncated `PATH` breaking every Homebrew
    /// binary; over-stripping is its own regression).
    #[test]
    fn agent_child_loses_the_daemon_credentials_and_keeps_everything_else() {
        let mut cmd = Command::new("printenv");
        cmd.env("BIOROUTER_SERVER__SECRET_KEY", "daemon-private")
            .env("BIOROUTER_ACP_WS_TOKEN", "daemon-private")
            .env("BIOROUTER_PORT", "4931")
            .env("SPOKEAGENT_PASSCODE", "extension-credential")
            .env("AWS_SECRET_ACCESS_KEY", "the-user's-own")
            .env("PATH", "/usr/bin");
        prepare_agent_child_command(&mut cmd);

        let envs: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let removed = |key: &str| envs.contains(&(key.to_string(), None));
        let kept =
            |key: &str, value: &str| envs.contains(&(key.to_string(), Some(value.to_string())));

        assert!(
            removed("BIOROUTER_SERVER__SECRET_KEY"),
            "the daemon's auth secret must not reach a child it spawns: {envs:?}"
        );
        assert!(
            removed("BIOROUTER_ACP_WS_TOKEN"),
            "the ACP token is a BioRouter credential too: {envs:?}"
        );
        assert!(kept("BIOROUTER_PORT", "4931"), "a port is not a credential");
        assert!(
            kept("SPOKEAGENT_PASSCODE", "extension-credential"),
            "an extension's declared credential is not the daemon's to strip"
        );
        assert!(
            kept("AWS_SECRET_ACCESS_KEY", "the-user's-own"),
            "the user's own credentials are not ours to censor"
        );
        assert!(kept("PATH", "/usr/bin"), "PATH must survive");
    }
}
