//! The refusal a command gives when it needs a person at a terminal and was run
//! without one (QA-D F9).
//!
//! `cliclack`'s prompts draw on **stderr** and read keys from stdin, or from
//! `/dev/tty` when stdin is not one (`console::read_single_key`). So the first
//! `.interact()` of `biorouter configure`, `biorouter project(s)` or `biorouter
//! session remove` failed, under a pipe, a cron job or an agent's shell, with
//! cliclack's bare `Error: not connected` — the `io::ErrorKind` name, which says
//! neither what happened nor what to run instead. And `configure` is the command
//! every install guide points at.
//!
//! ⚠ **Both streams are checked, not only stdin.** cliclack refuses on its own
//! when stderr is not a terminal (`PromptInteraction::interact_on`), whatever
//! stdin is, so a stdin-only check would still let `biorouter configure
//! 2>install.log` die with the old message. And when stdin is piped but a
//! controlling terminal exists, cliclack reads the keyboard through `/dev/tty`
//! and silently ignores what was piped — so `echo y | biorouter session remove
//! …` would sit waiting for a key the user thinks they already sent. Refusing
//! both cases with a sentence that names the alternative is the whole fix.

use std::io::IsTerminal;

/// The exit status of the refusal.
///
/// 2, the status clap exits with for a usage error, because this is the same
/// class of failure: the command was run in a way it cannot work, and nothing
/// was attempted. Distinct from 1 so a script can tell "you need `--yes`" from
/// "the removal itself failed".
pub const EXIT_CODE: u8 = 2;

/// A command that needed a person at a terminal was run without one.
///
/// A type rather than an `anyhow!` string so `main` can give it its own exit
/// status and print the sentence alone, without the `Error:` framing every
/// other failure gets: nothing failed — the command declined to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsTerminal(String);

impl std::fmt::Display for NeedsTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NeedsTerminal {}

/// Whether a cliclack prompt can run in this process: stdin and stderr must
/// both be terminals (see the module doc for why stderr counts too).
pub fn prompt_can_run() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// `Ok` when a prompt can run, otherwise the refusal carrying `sentence`.
///
/// `terminal` is a parameter, not a call to [`prompt_can_run`], so a test can
/// state the non-terminal case without redirecting its own stdin — and so each
/// caller samples the terminal once and decides from that one answer.
pub fn require(terminal: bool, sentence: &str) -> Result<(), NeedsTerminal> {
    if terminal {
        Ok(())
    } else {
        Err(NeedsTerminal(sentence.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_passes_and_anything_else_is_refused_with_the_sentence() {
        assert_eq!(require(true, "unused"), Ok(()));

        let refusal = require(false, "`biorouter x` is interactive.").unwrap_err();
        assert_eq!(refusal.to_string(), "`biorouter x` is interactive.");
    }

    /// `main` finds the refusal by downcasting the `anyhow::Error` it is handed,
    /// so the type must survive the `?` conversion and any `.context()` a caller
    /// wraps around it. If it did not, the refusal would fall back to exit 1
    /// with an `Error:` prefix — indistinguishable from a real failure.
    #[test]
    fn the_refusal_survives_conversion_into_anyhow_so_main_can_find_it() {
        let direct: anyhow::Error = require(false, "sentence").unwrap_err().into();
        assert!(direct.downcast_ref::<NeedsTerminal>().is_some());

        let wrapped = direct.context("while running a command");
        assert!(
            wrapped.downcast_ref::<NeedsTerminal>().is_some(),
            "a context layer must not hide the refusal from main's downcast"
        );
    }
}
