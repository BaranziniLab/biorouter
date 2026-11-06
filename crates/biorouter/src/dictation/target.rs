//! Who a dictation is aimed at (`docs/dictation/dictionary.md`, "The bias set", DD-30,
//! DD-77): whether its target chat is private, and the terminal rule that decides when a
//! CLI transcription may use private entries.
//!
//! [`chat_reach`] answers "is this chat private?" for every dictation door, so the stream
//! target, the observation stamp and the extractor stamp cannot disagree. The two uses fail
//! in opposite directions, each toward the safe side: a stamp treats `Unknown` as Private
//! (a stamp that fails toward private costs the person a publicize click), and a stream
//! target treats `Unknown` as public (a target that failed toward private would let private
//! terms shape text bound for a chat whose tier nobody knows).

use crate::privacy::{ProviderTier, SessionClassification};

use super::bias::{BiasTarget, Standing};

/// Whether a chat is private, as far as its stored row and its live model tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatReach {
    Private,
    Public,
    /// The chat's row could not be read.
    Unknown,
}

/// `stored` is `None` when the chat's row could not be read. `live_provider` is the tier of
/// the provider a live agent has bound, which can be Private before Gate B ratchets the
/// row: without it, a correction made in a chat bound to a private model before its first
/// turn would be stamped public.
pub fn chat_reach(
    stored: Option<SessionClassification>,
    live_provider: Option<ProviderTier>,
) -> ChatReach {
    if stored == Some(SessionClassification::Private)
        || live_provider.is_some_and(|t| t.is_private())
    {
        ChatReach::Private
    } else if stored.is_none() {
        ChatReach::Unknown
    } else {
        ChatReach::Public
    }
}

impl ChatReach {
    /// As a stream's target: only a known private chat is private.
    pub fn target_private(self) -> bool {
        self == ChatReach::Private
    }

    /// As a stamp: anything not known public is private.
    pub fn stamp_private(self) -> bool {
        self != ChatReach::Public
    }
}

/// What DD-77 reads for a CLI transcription, each sampled once by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CliTerminal {
    /// The `--session` chat's stored tier, read offline; `None` with no `--session`, or
    /// when the chat's row could not be read.
    pub session: Option<SessionClassification>,
    /// `prompt_can_run()`: stdin and stderr are both terminals.
    pub interactive: bool,
    /// `caller_capability()` is Private: the model this terminal is configured for.
    pub caller_private: bool,
    /// `BIOROUTER_TERMINAL` is set: an agent's shell.
    pub agent_shell: bool,
}

/// DD-77 for `transcribe` and `dictate --session <id>`: private entries only when the chat
/// is private **and** the run is at an interactive terminal **and** the configured model is
/// private **and** `BIOROUTER_TERMINAL` is unset. An agent's shell is public only whatever
/// the flags; without `--session` the transcription is public only.
///
/// The target's model affiliation is unstated: the chat's tier is read offline, with no
/// live agent to read a model from, so a private row that names an institution is left out
/// and only rows with no institution can reach it.
///
/// This is where a terminal's private standing is decided, the CLI half of the two places
/// that build a private [`Standing`].
pub fn cli_bias(terminal: CliTerminal) -> (BiasTarget, Standing) {
    let private = terminal.session == Some(SessionClassification::Private)
        && terminal.interactive
        && terminal.caller_private
        && !terminal.agent_shell;
    (
        BiasTarget {
            private,
            affiliation: None,
            person_included: false,
        },
        Standing::from_checked(private),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_reach_fails_toward_the_safe_side_of_each_use() {
        use ProviderTier as T;
        use SessionClassification as C;
        assert_eq!(chat_reach(Some(C::Private), None), ChatReach::Private);
        assert_eq!(
            chat_reach(Some(C::Public), Some(T::Private)),
            ChatReach::Private
        );
        assert_eq!(
            chat_reach(Some(C::Public), Some(T::Public)),
            ChatReach::Public
        );
        assert_eq!(chat_reach(None, None), ChatReach::Unknown);
        assert_eq!(chat_reach(None, Some(T::Private)), ChatReach::Private);
        assert!(!ChatReach::Unknown.target_private());
        assert!(ChatReach::Unknown.stamp_private());
        assert!(ChatReach::Private.target_private() && ChatReach::Private.stamp_private());
        assert!(!ChatReach::Public.target_private() && !ChatReach::Public.stamp_private());
    }

    /// DD-77: every condition is needed, and an agent's shell is public whatever the
    /// flags, as is a run without `--session`.
    #[test]
    fn the_cli_target_rule() {
        let all = CliTerminal {
            session: Some(SessionClassification::Private),
            interactive: true,
            caller_private: true,
            agent_shell: false,
        };
        let (target, standing) = cli_bias(all);
        assert!(target.private && standing.is_private());
        assert_eq!(
            target.affiliation, None,
            "a chat read offline states no model"
        );
        for broken in [
            CliTerminal {
                session: None,
                ..all
            },
            CliTerminal {
                session: Some(SessionClassification::Public),
                ..all
            },
            CliTerminal {
                interactive: false,
                ..all
            },
            CliTerminal {
                caller_private: false,
                ..all
            },
            CliTerminal {
                agent_shell: true,
                ..all
            },
        ] {
            let (target, standing) = cli_bias(broken);
            assert!(!target.private && !standing.is_private(), "{broken:?}");
        }
    }
}
