//! A sign-in card in a terminal chat on the shared daemon (the owner's D1 ruling on AO-12): the
//! raw-mode sign-in terminal ([`crate::daemon_client::CrewClient::authenticate_ssh_at`]) on
//! `/crew/setup/{job}/authentication` or `/crew/connections/{id}/authentication`, after a header
//! the daemon's fields fill.
//!
//! What the person types goes from this keyboard to ssh on the server through the daemon's
//! sign-in terminal, with the person's proof. It is never printed by this module, posted to the
//! card or added to the chat. The card is never answered as done: the daemon takes it down when
//! the job moves on or the sign-in is adopted. When the terminal ends with nothing signed in,
//! the card is dismissed, so the chat carries on rather than waiting for a window that closed.
//!
//! ⚠ **The person opens the sign-in; a card never opens it by itself.** On the desktop the Sign in window opens
//! only when the person presses Sign in. Here the card asks first ([`question`]), and only
//! Enter with nothing typed opens ssh's prompts. A card arrives when the model calls for it, so
//! opening at once would send whatever the person was typing (often their next message) to the
//! server's password prompt as a failed attempt. Skipping dismisses the card, as Not now does on
//! the desktop.

use super::StepSurface;
use crate::daemon_client::CrewClient;
use anyhow::Result;
use biorouter::conversation::message::{CrewSignInTarget, CrewStepRequest};
use serde_json::{json, Value};
use std::io::IsTerminal;
use zeroize::Zeroizing;

/// Where to sign in when this terminal can't: the desktop app, or `biorouter crew auth`.
pub(crate) fn fallback(step: &CrewStepRequest) -> Option<String> {
    let CrewStepRequest::SignIn { target, login, .. } = step else {
        return None;
    };
    Some(match target {
        CrewSignInTarget::Setup { job_id } => format!(
            "Sign in to {login} in the Biorouter desktop app, where this setup waits with a Sign in button, or run biorouter crew auth --setup-job {job_id} in a terminal. What you type goes to ssh, never to the chat."
        ),
        CrewSignInTarget::Connection { connection_id } => format!(
            "Sign in to {login} in the Biorouter desktop app, with Sign in on the connection in Crew, or run biorouter crew auth --connection {connection_id} in a terminal. What you type goes to ssh, never to the chat."
        ),
    })
}

/// The header printed before the sign-in terminal opens, from the daemon's own login.
pub(crate) fn header(login: &str) -> String {
    format!("Signing in to {login}. What you type goes to ssh, never to the chat.")
}

/// Asked before anything opens, with the daemon's own login.
pub(crate) fn question(login: &str) -> String {
    format!("Sign in to {login} now? Press Enter to open the sign-in, or type n to skip: ")
}

/// Said when a line answers neither way. What was typed is not repeated or sent anywhere.
const ANSWER_AGAIN: &str = "That opened nothing and went nowhere. Press Enter with nothing typed to open the sign-in, or type n to skip.";

/// How many answers that say neither are taken before the card is skipped.
pub(crate) const ASKS: usize = 3;

/// The most an answer to [`question`] may be, in bytes. Large, so a whole message typed before
/// the card arrived is read as one line that answers neither way, rather than cut short with
/// its end left to answer the next question.
pub(crate) const ANSWER_BYTES: usize = 4096;

/// What a line typed at [`question`] chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Enter with nothing typed.
    Open,
    /// `n` or `no`, Esc, Ctrl-C, or a terminal that could not be read.
    Skip,
    /// Anything else, such as a message typed before the card arrived.
    Again,
}

pub(crate) fn choice(typed: Option<&str>) -> Choice {
    match typed.map(str::trim) {
        Some("") => Choice::Open,
        Some(text) if text.eq_ignore_ascii_case("n") || text.eq_ignore_ascii_case("no") => {
            Choice::Skip
        }
        Some(_) => Choice::Again,
        None => Choice::Skip,
    }
}

/// Said, with the code `crew_needs_terminal`, when this chat has no terminal to show the
/// server's prompts in.
pub(crate) fn needs_terminal(step: &CrewStepRequest) -> String {
    let mut sentence =
        "Signing in needs a terminal, to show the server's password and verification-code prompts."
            .to_owned();
    if let Some(elsewhere) = fallback(step) {
        sentence.push(' ');
        sentence.push_str(&elsewhere);
    }
    sentence
}

/// Said once the terminal signed the person in.
const SIGNED_IN: &str = "Signed in. The chat carries on.";
/// Said once the terminal ended with nothing signed in.
const NOT_SIGNED_IN: &str = "Nothing was signed in, so the chat carries on without it.";

/// What a sign-in card needs of this terminal and the shared daemon: the person's answer to
/// [`question`], the daemon's sign-in terminal and the card's dismissal. A trait so a test can
/// stand in for both.
pub(crate) trait SignInDaemon {
    /// One line the person types after `question`, `Ok(None)` for Esc or Ctrl-C. Zeroized,
    /// because a person may type a password here before ssh asks for one.
    async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>>;
    /// The raw-mode sign-in terminal the daemon prepares at `prepare_path`, refused with
    /// `no_terminal` when there is no terminal here.
    async fn sign_in_at(&self, prepare_path: &str, no_terminal: &str) -> Result<Value>;
    /// Dismiss the card `card_id` in the chat `session_id` (`POST /action-required/crew-step`).
    async fn dismiss(&self, session_id: &str, card_id: &str) -> Result<()>;
}

impl SignInDaemon for CrewClient {
    async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>> {
        crate::commands::shared_conversation::terminal_input(question, false, ANSWER_BYTES).await
    }

    async fn sign_in_at(&self, prepare_path: &str, no_terminal: &str) -> Result<Value> {
        self.authenticate_ssh_at(prepare_path, no_terminal).await
    }

    async fn dismiss(&self, session_id: &str, card_id: &str) -> Result<()> {
        self.request(
            "POST",
            "/action-required/crew-step",
            Some(json!({"id": card_id, "session_id": session_id, "outcome": "dismiss"})),
        )
        .await
        .map(|_| ())
    }
}

/// Take a sign-in card: the question, then, once the person chose to, the header and this
/// terminal's sign-in to the card's target.
pub(crate) async fn take(surface: &StepSurface<'_>, step: &CrewStepRequest) -> Result<()> {
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut say = |line: &str| surface.say(line);
    run(
        surface.client,
        &mut say,
        surface.session_id,
        surface.card_id,
        step,
        terminal,
    )
    .await
}

/// Whether the person chose to open the sign-in: [`question`] asked up to [`ASKS`] times, and
/// only Enter with nothing typed says yes. A terminal that could not be read says no.
async fn person_opens(
    daemon: &impl SignInDaemon,
    say: &mut (dyn FnMut(&str) -> Result<()> + Send),
    login: &str,
) -> Result<bool> {
    for asked in 1..=ASKS {
        let typed = daemon.ask(&question(login)).await.unwrap_or(None);
        match choice(typed.as_ref().map(|typed| typed.as_str())) {
            Choice::Open => return Ok(true),
            Choice::Skip => return Ok(false),
            Choice::Again if asked < ASKS => say(ANSWER_AGAIN)?,
            Choice::Again => {}
        }
    }
    Ok(false)
}

/// [`take`], against `daemon`, saying each line with `say`. `terminal` says whether a person can
/// answer ssh's prompts here; without one the card is refused with `crew_needs_terminal`, as
/// `biorouter crew auth` refuses, and left for the desktop app to take. With one, nothing opens
/// until the person chooses to ([`person_opens`]); a skip dismisses the card.
async fn run(
    daemon: &impl SignInDaemon,
    say: &mut (dyn FnMut(&str) -> Result<()> + Send),
    session_id: &str,
    card_id: &str,
    step: &CrewStepRequest,
    terminal: bool,
) -> Result<()> {
    let CrewStepRequest::SignIn { target, login, .. } = step else {
        return Ok(());
    };
    let no_terminal = needs_terminal(step);
    crate::commands::needs_terminal::require(terminal, &no_terminal)?;
    if !person_opens(daemon, say, login).await? {
        let _ = daemon.dismiss(session_id, card_id).await;
        return say(NOT_SIGNED_IN);
    }
    let path = match target {
        CrewSignInTarget::Setup { job_id } => format!("/crew/setup/{job_id}/authentication"),
        CrewSignInTarget::Connection { connection_id } => {
            format!("/crew/connections/{connection_id}/authentication")
        }
    };
    say(&header(login))?;
    let signed_in = match daemon.sign_in_at(&path, &no_terminal).await {
        Ok(ended) => ended["authenticated"].as_bool() == Some(true),
        Err(error) => {
            if error
                .chain()
                .any(|cause| cause.is::<crate::commands::needs_terminal::NeedsTerminal>())
            {
                return Err(error);
            }
            say(&error.to_string())?;
            false
        }
    };
    if signed_in {
        return say(SIGNED_IN);
    }
    // The daemon may already have taken the card down (a sign-in adopted as it ended); then
    // there is nothing to dismiss, and that is not a failure.
    let _ = daemon.dismiss(session_id, card_id).await;
    say(NOT_SIGNED_IN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// One answer the person gives at the question.
    #[derive(Clone, Copy)]
    enum Typed {
        Line(&'static str),
        Escape,
        Unreadable,
    }

    /// The terminal and the shared daemon, as a sign-in card sees them: the answers typed at the
    /// question in order (Enter when none are left), what was asked of the daemon, and what it
    /// answers.
    #[derive(Default)]
    struct Daemon {
        signed_in: Option<bool>,
        refusal: Option<String>,
        typed: Mutex<Vec<Typed>>,
        questions: Mutex<Vec<String>>,
        asked: Mutex<Vec<String>>,
    }

    impl Daemon {
        fn answering(typed: &[Typed]) -> Self {
            Self {
                typed: Mutex::new(typed.iter().rev().copied().collect()),
                ..Self::default()
            }
        }
    }

    impl SignInDaemon for Daemon {
        async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>> {
            self.questions.lock().unwrap().push(question.to_owned());
            let typed = self.typed.lock().unwrap().pop().unwrap_or(Typed::Line(""));
            match typed {
                Typed::Line(line) => Ok(Some(Zeroizing::new(line.to_owned()))),
                Typed::Escape => Ok(None),
                Typed::Unreadable => anyhow::bail!("Human input requires an interactive terminal"),
            }
        }

        async fn sign_in_at(&self, prepare_path: &str, _no_terminal: &str) -> Result<Value> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("sign in at {prepare_path}"));
            if let Some(refusal) = &self.refusal {
                anyhow::bail!("{refusal}");
            }
            Ok(json!({"authentication_id": "a", "exit_code": 0,
                "authenticated": self.signed_in.unwrap_or(false)}))
        }

        async fn dismiss(&self, session_id: &str, card_id: &str) -> Result<()> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("dismiss {card_id} in {session_id}"));
            Ok(())
        }
    }

    fn setup_step() -> CrewStepRequest {
        CrewStepRequest::SignIn {
            target: CrewSignInTarget::Setup {
                job_id: "job-1".into(),
            },
            server: "hpc.example.edu".into(),
            login: "you@hpc.example.edu".into(),
        }
    }

    async fn taken(
        daemon: &Daemon,
        step: &CrewStepRequest,
        terminal: bool,
    ) -> (Result<()>, Vec<String>) {
        let mut said = Vec::new();
        let mut say = |line: &str| {
            said.push(line.to_owned());
            Ok(())
        };
        let result = run(daemon, &mut say, "chat-1", "card-1", step, terminal).await;
        (result, said)
    }

    #[tokio::test]
    async fn a_setup_card_signs_in_on_the_job_s_path_after_the_header() {
        let daemon = Daemon {
            signed_in: Some(true),
            ..Daemon::default()
        };
        let (result, said) = taken(&daemon, &setup_step(), true).await;
        result.unwrap();
        assert_eq!(
            *daemon.asked.lock().unwrap(),
            ["sign in at /crew/setup/job-1/authentication"]
        );
        assert_eq!(
            said,
            [
                "Signing in to you@hpc.example.edu. What you type goes to ssh, never to the chat.",
                SIGNED_IN,
            ]
        );
    }

    #[tokio::test]
    async fn a_connection_card_signs_in_on_the_connection_s_path() {
        let daemon = Daemon {
            signed_in: Some(true),
            ..Daemon::default()
        };
        let step = CrewStepRequest::SignIn {
            target: CrewSignInTarget::Connection {
                connection_id: "conn-1".into(),
            },
            server: "lab.example.edu".into(),
            login: "me@lab.example.edu".into(),
        };
        taken(&daemon, &step, true).await.0.unwrap();
        assert_eq!(
            *daemon.asked.lock().unwrap(),
            ["sign in at /crew/connections/conn-1/authentication"]
        );
    }

    /// A terminal that ended with nothing signed in, or that the daemon refused, dismisses the
    /// card, so the chat is not left waiting for a window that closed.
    #[tokio::test]
    async fn nothing_signed_in_dismisses_the_card() {
        let daemon = Daemon::default();
        let (result, said) = taken(&daemon, &setup_step(), true).await;
        result.unwrap();
        assert_eq!(
            *daemon.asked.lock().unwrap(),
            [
                "sign in at /crew/setup/job-1/authentication",
                "dismiss card-1 in chat-1"
            ]
        );
        assert_eq!(said.last().unwrap(), NOT_SIGNED_IN);

        let refused = Daemon {
            refusal: Some(
                "This server already has a sign-in open; close it before opening another".into(),
            ),
            ..Daemon::default()
        };
        let (result, said) = taken(&refused, &setup_step(), true).await;
        result.unwrap();
        assert!(
            said.iter()
                .any(|line| line.contains("already has a sign-in open")),
            "{said:?}"
        );
        assert_eq!(refused.asked.lock().unwrap().len(), 2);
    }

    /// No terminal: refused with the usage refusal (`crew_needs_terminal`), nothing asked of the
    /// daemon and nothing dismissed, so the desktop app can still take the card.
    #[tokio::test]
    async fn without_a_terminal_the_card_is_refused_and_left() {
        let daemon = Daemon::default();
        let (result, said) = taken(&daemon, &setup_step(), false).await;
        let error = result.unwrap_err();
        assert!(error.is::<crate::commands::needs_terminal::NeedsTerminal>());
        let sentence = error.to_string();
        assert!(
            sentence.starts_with("Signing in needs a terminal"),
            "{sentence}"
        );
        assert!(
            sentence.contains("biorouter crew auth --setup-job job-1"),
            "{sentence}"
        );
        assert!(daemon.asked.lock().unwrap().is_empty());
        assert!(daemon.questions.lock().unwrap().is_empty());
        assert!(said.is_empty());
    }

    /// A card arrives when the model calls for it, so nothing opens until the person presses
    /// Enter at the question: a message typed before it arrived opens nothing and is not
    /// repeated, and `n` skips, dismissing the card as Not now does on the desktop.
    #[tokio::test]
    async fn nothing_opens_until_the_person_chooses_to() {
        let daemon =
            Daemon::answering(&[Typed::Line("what's next for the cohort?"), Typed::Line("n")]);
        let (result, said) = taken(&daemon, &setup_step(), true).await;
        result.unwrap();
        assert_eq!(
            *daemon.asked.lock().unwrap(),
            ["dismiss card-1 in chat-1"],
            "no sign-in was opened"
        );
        assert_eq!(said, [ANSWER_AGAIN, NOT_SIGNED_IN]);
        assert!(!said.iter().any(|line| line.contains("cohort")), "{said:?}");
        let questions = daemon.questions.lock().unwrap().clone();
        assert_eq!(questions.len(), 2);
        assert_eq!(
            questions[0],
            "Sign in to you@hpc.example.edu now? Press Enter to open the sign-in, or type n to skip: "
        );

        // Esc skips, and so does a terminal that could not be read.
        for typed in [Typed::Escape, Typed::Unreadable] {
            let daemon = Daemon::answering(&[typed]);
            let (result, said) = taken(&daemon, &setup_step(), true).await;
            result.unwrap();
            assert_eq!(*daemon.asked.lock().unwrap(), ["dismiss card-1 in chat-1"]);
            assert_eq!(said, [NOT_SIGNED_IN]);
        }
    }

    /// An answer that says neither is asked again, [`ASKS`] times at most, and then the card is
    /// skipped; Enter after one opens the sign-in.
    #[tokio::test]
    async fn an_answer_that_says_neither_is_asked_again_then_skipped() {
        let daemon =
            Daemon::answering(&[Typed::Line("yes"), Typed::Line("ok"), Typed::Line("sure")]);
        let (result, said) = taken(&daemon, &setup_step(), true).await;
        result.unwrap();
        assert_eq!(daemon.questions.lock().unwrap().len(), ASKS);
        assert_eq!(*daemon.asked.lock().unwrap(), ["dismiss card-1 in chat-1"]);
        assert_eq!(said, [ANSWER_AGAIN, ANSWER_AGAIN, NOT_SIGNED_IN]);

        let daemon = Daemon {
            signed_in: Some(true),
            ..Daemon::answering(&[Typed::Line("hello"), Typed::Line("  ")])
        };
        let (result, said) = taken(&daemon, &setup_step(), true).await;
        result.unwrap();
        assert_eq!(
            *daemon.asked.lock().unwrap(),
            ["sign in at /crew/setup/job-1/authentication"]
        );
        assert_eq!(said.last().unwrap(), SIGNED_IN);
    }

    #[test]
    fn the_words_name_where_to_sign_in_and_use_no_dashes() {
        let setup = fallback(&setup_step()).unwrap();
        assert!(setup.contains("you@hpc.example.edu"), "{setup}");
        assert!(setup.contains("desktop app"), "{setup}");
        assert!(
            setup.contains("biorouter crew auth --setup-job job-1"),
            "{setup}"
        );
        let connection = fallback(&CrewStepRequest::SignIn {
            target: CrewSignInTarget::Connection {
                connection_id: "conn-1".into(),
            },
            server: "hpc.example.edu".into(),
            login: "you@hpc.example.edu".into(),
        })
        .unwrap();
        assert!(
            connection.contains("biorouter crew auth --connection conn-1"),
            "{connection}"
        );
        for text in [
            setup,
            connection,
            header("you@hpc.example.edu"),
            question("you@hpc.example.edu"),
            ANSWER_AGAIN.to_owned(),
            needs_terminal(&setup_step()),
            SIGNED_IN.to_owned(),
            NOT_SIGNED_IN.to_owned(),
        ] {
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
            assert!(!text.to_lowercase().contains("device"), "{text}");
        }
    }
}
