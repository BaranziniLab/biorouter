//! A Crew step card in a terminal chat on the shared daemon (`biorouter session --shared-daemon`,
//! `session send`, `session attach`): the owner's D1 ruling on AO-12, which lets a chat open the
//! person's own Sign in window, vault prompt or host-key help.
//!
//! The card carries the daemon's own fields and nothing a value could sit in
//! ([`CrewStepRequest`]). What the person types here goes from this terminal straight to the
//! daemon: ssh's prompts through the raw-mode sign-in terminal
//! ([`crate::daemon_client::CrewClient::authenticate_ssh_at`]), a vault passphrase through the
//! hidden prompt ([`crate::daemon_client::read_secret`]) to the vault route with the person's
//! proof. Nothing typed is printed, added to the conversation or posted to the card, which only
//! a dismissal answers (`POST /action-required/crew-step`). Each step's own module says, where
//! it cannot take the step here, where to take it instead: the desktop app, or the
//! `biorouter crew` command that does it.

pub(crate) mod host_key;
pub(crate) mod sign_in;
pub(crate) mod vault;

use crate::daemon_client::CrewClient;
use anyhow::Result;
use biorouter::conversation::message::CrewStepRequest;
use std::io::Write;

/// The chat a Crew step card arrived in, as a step needs it.
pub(crate) struct StepSurface<'a> {
    /// The shared daemon, with the person's proof.
    pub(crate) client: &'a CrewClient,
    /// The chat the card is in.
    pub(crate) session_id: &'a str,
    /// The card's ID, which a dismissal posts back.
    pub(crate) card_id: &'a str,
}

impl StepSurface<'_> {
    /// One line for the person, on the chat's notice channel (stderr), with anything a
    /// terminal could act on removed. Never a value the person typed.
    pub(crate) fn say(&self, line: &str) -> Result<()> {
        let mut stderr = std::io::stderr().lock();
        writeln!(
            stderr,
            "{}",
            crate::commands::shared_conversation::safe_text(line)
        )?;
        stderr.flush()?;
        Ok(())
    }
}

/// Take a Crew step card in an interactive terminal chat: the step's own module does it. The
/// caller has already checked that a person is at this terminal; a chat with nobody at it
/// refuses before this is called, as it does for every card.
pub(crate) async fn take(surface: &StepSurface<'_>, step: &CrewStepRequest) -> Result<()> {
    match step {
        CrewStepRequest::SignIn { .. } => sign_in::take(surface, step).await,
        CrewStepRequest::Vault { .. } => vault::take(surface, step).await,
        CrewStepRequest::HostKey { .. } => host_key::take(surface, step).await,
    }
}
