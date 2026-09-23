//! The Linux prompter (DR-24): polkit, via `pkcheck` against BioRouter's own
//! action.
//!
//! ⚠ **This module is compiled on every target, not just Linux**, and that is
//! deliberate. It talks to polkit by spawning a process, so every line of it is
//! ordinary `std`/`tokio` code that a macOS or Windows compiler checks just as
//! well as a Linux one — and this campaign has no Linux host to build on.
//! Compiling it everywhere means a typo here is a red build on the developer's
//! Mac instead of a broken Linux release. [`super::system_auth::platform_prompter`]
//! is what restricts it to Linux; nothing else selects it.
//!
//! # Why `pkcheck` against our own action, and not `pkexec`
//!
//! `pkexec` is the obvious reach, and it is wrong twice over. It runs a program
//! **as root** merely to verify a user, and its built-in action
//! `org.freedesktop.policykit.exec` ships as `auth_admin_keep` — `_keep` caches
//! the authorization for several minutes, so the second declassification inside
//! that window would raise no prompt at all. DR-20 point 2 admits no cached
//! grant. [`super::system_auth::POLKIT_ACTION_POLICY`] therefore declares a
//! dedicated `auth_self` action: the user's own password, every time, and no
//! privileged execution anywhere.
//!
//! # What is not done yet
//!
//! No packaging path installs the action file, so on a real Linux install today
//! `pkcheck` reports the action as unknown and this prompter returns
//! [`AuthOutcome::Unavailable`]. DR-24 rules that outcome acceptable and a
//! silent approval unacceptable, so this fails in the direction the ruling
//! names — but a Linux user cannot declassify until the deb and rpm makers ship
//! `edu.ucsf.biorouter.declassify.policy` into `/usr/share/polkit-1/actions/`.

use super::system_auth::{
    outcome_for_polkit, AuthOutcome, AuthRequest, PolkitSignal, SystemAuthenticator,
    POLKIT_ACTION_ID,
};
use std::io::IsTerminal;

pub struct PolkitPrompter;

pub fn prompter() -> &'static dyn SystemAuthenticator {
    &PolkitPrompter
}

#[async_trait::async_trait]
impl SystemAuthenticator for PolkitPrompter {
    async fn authenticate(&self, req: &AuthRequest) -> AuthOutcome {
        outcome_for_polkit(check_authorization(req).await)
    }

    fn platform(&self) -> &'static str {
        "Linux (polkit)"
    }

    fn unavailable_reason(&self) -> String {
        format!(
            "Biorouter could not raise a system authentication prompt on {}, so this \
             operation was refused and nothing was changed. polkit must be installed, \
             this session must be able to show a prompt, and the action \
             '{POLKIT_ACTION_ID}' must be registered in \
             /usr/share/polkit-1/actions/. Headless hosts have no prompter; run this \
             from a Biorouter session on a desktop machine.",
            self.platform()
        )
    }
}

/// Where `pkcheck` is allowed to be, in order.
///
/// ⚠ **Absolute paths, never a PATH lookup.** This used to be
/// `Command::new("pkcheck")`, which resolves through `PATH` — and `PATH` is
/// part of the daemon's environment, which AR-11 measured to be recoverable and
/// settable in-process. So anything running as this user could drop a
/// `pkcheck` of its own earlier in `PATH`, exit 0, and the daemon would read
/// that as "the user authenticated", with no polkit dialog ever shown. The same
/// class of hole as trusting a file another same-user process can write; the
/// fix is to name the binary rather than ask the environment where it is.
///
/// Both locations are root-owned on every distribution that ships polkit, so
/// substituting one is a privilege the attacker would already need to have.
const PKCHECK_PATHS: &[&str] = &["/usr/bin/pkcheck", "/bin/pkcheck"];

/// The `pkcheck` this host actually has, or `None`.
fn pkcheck_binary() -> Option<&'static str> {
    PKCHECK_PATHS
        .iter()
        .copied()
        .find(|path| std::path::Path::new(path).exists())
}

/// Ask polkit whether this process is authorised, letting it prompt.
async fn check_authorization(req: &AuthRequest) -> PolkitSignal {
    let Some(pkcheck) = pkcheck_binary() else {
        return PolkitSignal::NotInstalled;
    };
    if !can_show_a_prompt() {
        return PolkitSignal::NoAuthenticationAgent;
    }

    let mut command = tokio::process::Command::new(pkcheck);
    // This module compiles on every target (see the header), so it is reached by
    // the console-window census on Windows too. `pkcheck` only ever RUNS on
    // Linux, where the flag is a no-op — it is set so the site carries its own
    // answer instead of leaning on an exemption row whose reason was a naming
    // argument rather than a `cfg` fact (#368).
    biorouter_mcp::developer::shell::no_console_window(&mut command);
    let output = command
        .arg("--action-id")
        .arg(POLKIT_ACTION_ID)
        .arg("--process")
        .arg(std::process::id().to_string())
        // Without this, polkit answers "authorization required" instead of
        // asking — the flag is what makes this a prompt rather than a query.
        .arg("--allow-user-interaction")
        // DR-20 point 4: the dialog must name what it authorises. polkit
        // interpolates `$(chats)` in the action's message from this detail.
        .arg("--detail")
        .arg("chats")
        .arg(req.session_ids.join(", "))
        .output()
        .await;

    match output {
        Ok(done) => match done.status.code() {
            Some(code) => PolkitSignal::Exited(code),
            // Killed by a signal: no answer, so no approval.
            None => PolkitSignal::Signalled,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PolkitSignal::NotInstalled,
        // Anything else that stopped `pkcheck` running is "no prompt happened",
        // which is pkcheck's own exit code 2.
        Err(_) => PolkitSignal::Exited(2),
    }
}

/// Whether anything on this machine could draw a polkit prompt.
///
/// A graphical session has an agent; a terminal can run the text agent. A daemon
/// with neither would sit at `pkcheck` until it timed out, and reporting
/// [`PolkitSignal::NoAuthenticationAgent`] up front turns that hang into an
/// immediate, explained refusal.
fn can_show_a_prompt() -> bool {
    std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_host_without_polkit_is_unavailable_and_never_approves() {
        // On this developer machine (and on any headless Linux host) `pkcheck`
        // is absent, so this exercises the real acquisition path end to end and
        // asserts the honest DR-24 outcome rather than a mocked one.
        let req = AuthRequest::new("Make 1 chat public.", &["20260804_120000".into()]).unwrap();
        let outcome = PolkitPrompter.authenticate(&req).await;
        assert_ne!(
            outcome,
            AuthOutcome::Approved,
            "a machine with no polkit must never approve"
        );
        // Mirrors the production lookup exactly — a test that asked `PATH`
        // while the code asks two absolute paths would drift from it silently.
        if pkcheck_binary().is_none() {
            assert_eq!(outcome, AuthOutcome::Unavailable);
        }
    }

    /// ⚠ **`pkcheck` must never be resolved through `PATH`.** `PATH` lives in
    /// the daemon's environment, which AR-11 measured to be settable
    /// in-process, so a `Command::new("pkcheck")` is a bypass any same-user
    /// process can arrange for itself: drop a `pkcheck` earlier in `PATH`, exit
    /// 0, and the daemon reads that as "the user authenticated" with no polkit
    /// dialog shown at all.
    #[test]
    fn the_polkit_binary_is_named_absolutely_and_not_looked_up() {
        // ⚠ Comment lines are stripped first. Both the fix's own docstring and
        // this test's explain the bug by quoting it, so a naive `contains`
        // matches its own prose and fails on correct code — the same
        // self-referential trap this repo hit when a comment quoted the class
        // name a gate was counting.
        let code: String = include_str!("system_auth_polkit.rs")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("Command::new(\"pkcheck\")"),
            "pkcheck must be invoked by absolute path, never resolved through PATH"
        );
        for path in PKCHECK_PATHS {
            assert!(
                path.starts_with('/'),
                "{path} is not absolute, so it would be resolved through PATH"
            );
        }
    }

    #[test]
    fn the_refusal_names_polkit_the_action_and_the_platform() {
        let reason = PolkitPrompter.unavailable_reason();
        assert!(reason.contains("polkit"), "got {reason}");
        assert!(reason.contains(POLKIT_ACTION_ID), "got {reason}");
        assert!(reason.contains("Linux"), "got {reason}");
    }
}
