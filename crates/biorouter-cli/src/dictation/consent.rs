//! Who may open the microphone (DD-73, DD-76, P-7), and who may let private dictionary terms
//! shape a recording headed for an unknown destination (DD-77, O-8).
//!
//! [`CaptureConsent`] is the only key to a recording: [`super::capture::CaptureChild::start`]
//! takes one, and it is built here and nowhere else, after the two checks that tell a
//! person from an agent: `BIOROUTER_TERMINAL` is unset (every agent shell sets it) and
//! stdin and stderr are terminals (`prompt_can_run()`). Its one field is private to this
//! module, so no other code can build it, and `capture_consent_is_minted_in_one_file` reads
//! the crate to hold it so. A capture without a terminal is what an agent's tool call
//! looks like (O-20). These checks stop mistakes by a cooperative agent; an agent that
//! unsets the variable and fakes a terminal is not stopped, which DD-20 accepts for the
//! same reason.

use biorouter::dictation::{cli_bias, BiasTarget, CliTerminal, Standing};
use biorouter::privacy::SessionClassification;

use super::environment::CaptureVerdict;
use super::messages;

/// Set in an agent's shell. Any value but an empty one counts, as for `dictation` and
/// `dictionary`.
pub const AGENT_SHELL_ENV: &str = "BIOROUTER_TERMINAL";

/// Proof that a person at this terminal started this recording. It has no data: holding
/// one is the whole point.
#[derive(Debug)]
pub struct CaptureConsent {
    _person_at_this_terminal: (),
}

impl CaptureConsent {
    /// Steps 3 and 4 of the detection order, sampled now.
    pub fn at_this_terminal() -> Result<CaptureConsent, CaptureVerdict> {
        if std::env::var_os(AGENT_SHELL_ENV).is_some_and(|v| !v.is_empty()) {
            return Err(CaptureVerdict::AgentShell);
        }
        if !crate::commands::needs_terminal::prompt_can_run() {
            return Err(CaptureVerdict::NoTerminal);
        }
        Ok(CaptureConsent {
            _person_at_this_terminal: (),
        })
    }
}

/// What `--private-terms` asked and what the terminal can vouch for (O-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateTermsAsk {
    /// `prompt_can_run()`.
    pub interactive: bool,
    /// `BIOROUTER_TERMINAL` set.
    pub agent_shell: bool,
    /// `caller_capability()` is Private: a private model is configured on this computer.
    pub caller_private: bool,
}

/// Whether `--private-terms` may even be asked about: at a person's own terminal, outside
/// an agent's shell, with a private model configured. Otherwise MSG-PRIVATE-TERMS-REFUSED.
pub fn private_terms_admitted(ask: PrivateTermsAsk) -> bool {
    ask.interactive && !ask.agent_shell && ask.caller_private
}

/// The bias of a recording the person chose to include their private terms in, after they
/// answered `y` to MSG-PRIVATE-TERMS: every private row, institution tagged rows included
/// (O-37), and nothing about the destination becomes private. The standing comes from the
/// terminal rule of DD-77, `cli_bias`, which checks the terminal, the configured model and
/// the agent's shell itself; the person's yes stands where a private chat's tier would.
pub fn included_private_terms(ask: PrivateTermsAsk) -> (BiasTarget, Standing) {
    let (mut target, standing) = cli_bias(CliTerminal {
        session: Some(SessionClassification::Private),
        interactive: ask.interactive,
        caller_private: ask.caller_private,
        agent_shell: ask.agent_shell,
    });
    if !standing.is_private() {
        return (BiasTarget::public(), Standing::public());
    }
    target.private = false;
    target.affiliation = None;
    target.person_included = true;
    (target, standing)
}

/// MSG-PRIVATE-TERMS at the terminal, default No. Reads one line from stdin, which is a
/// terminal here.
pub fn ask_private_terms() -> bool {
    use std::io::{BufRead, Write};
    let mut err = std::io::stderr();
    let _ = write!(err, "{} [y/N] ", messages::MSG_PRIVATE_TERMS);
    let _ = err.flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    is_yes(&answer)
}

/// `y` or `yes`, any case; everything else, an empty line included, is No.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P-7: `CaptureConsent` is built in this file alone. The field is private, so another
    /// module cannot write the literal; this reads every source file of the crate so a
    /// second constructor here, or the field made public, is caught too.
    #[test]
    fn capture_consent_is_minted_in_one_file() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        let mut todo = vec![src.clone()];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        assert!(files.len() > 20, "the walk read {} files", files.len());
        // Spelled in two pieces, so this line is not one of the matches.
        let field = ["_person_at_this", "_terminal"].concat();
        let literal = format!("{field}: ()");
        let here = std::path::Path::new("dictation").join("consent.rs");
        let mut minted = Vec::new();
        let mut definition = String::new();
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let relative = path.strip_prefix(&src).unwrap().to_path_buf();
            if relative == here {
                definition = text.clone();
            }
            let count = text.matches(&literal).count();
            if count > 0 {
                minted.push((relative.display().to_string(), count));
            }
        }
        // The definition and the one constructor in `at_this_terminal`.
        assert_eq!(
            minted,
            vec![(here.display().to_string(), 2)],
            "CaptureConsent is built outside consent.rs, or twice in it"
        );
        assert!(
            definition.contains(&format!(
                "pub struct CaptureConsent {{\n    {field}: (),\n}}"
            )),
            "CaptureConsent's one field must stay private"
        );
    }

    /// DD-77 and O-8: `--private-terms` is admitted only at a person's terminal, outside an
    /// agent's shell, with a private model configured.
    #[test]
    fn private_terms_need_a_person_and_a_private_model() {
        let ask = PrivateTermsAsk {
            interactive: true,
            agent_shell: false,
            caller_private: true,
        };
        assert!(private_terms_admitted(ask));
        for refused in [
            PrivateTermsAsk {
                interactive: false,
                ..ask
            },
            PrivateTermsAsk {
                agent_shell: true,
                ..ask
            },
            PrivateTermsAsk {
                caller_private: false,
                ..ask
            },
        ] {
            assert!(!private_terms_admitted(refused), "{refused:?}");
            let (target, standing) = included_private_terms(refused);
            assert!(!standing.is_private(), "{refused:?}");
            assert_eq!(target, BiasTarget::public());
        }
        let (target, standing) = included_private_terms(ask);
        assert!(standing.is_private());
        assert!(target.person_included);
        assert!(!target.private, "the destination is unknown, not private");
    }

    #[test]
    fn only_yes_is_yes() {
        for yes in ["y", "Y", "yes", " YES\n"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "\n", "n", "no", "sure", "yy"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }
}
