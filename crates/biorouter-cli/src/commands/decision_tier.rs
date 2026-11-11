//! DR-33 at the terminal: what a decision-model save does to a chat's privacy
//! tier, said before it is written, and the confirmation a person types when
//! it makes the chat public.
//!
//! A chat is as private as its least private model, decision models
//! included. So a Versa chat that names a Baseten decision model before its
//! first message is a public chat, and saving that name is a lowering. Only a
//! person may lower a chat (design §1.8), and they must say so:
//!
//! * through a daemon, the save carries the person's acceptance
//!   (`acceptPublicDecisionModels`) only after the daemon answered 409
//!   `needsPublicDecisionConfirmation` and the person confirmed here;
//! * with no daemon, the local save asks the same question.
//!
//! Either way the confirmation is **typed at a terminal**: the last six
//! characters of the chat id, as `biorouter session declassify` asks. With no
//! terminal on stdin and stdout (an agent's shell, a script, a pipe) the save
//! is refused and nothing is written. There is deliberately no flag that
//! answers the question in advance: a flag is exactly what a prompt-injected
//! shell would type (design §10, leaks 2).
//!
//! ⚠ The typed answer is not declassification's proof of a person (the
//! `privacy::declassify` type `session declassify` mints): a census pins the
//! two files that may even name it, and this would be a third. A lowering is checked by its own one writer
//! (`SessionManager::write_public_basis`), which takes no proof. What this
//! module guarantees is that no lowering leaves this terminal without the
//! person having read the effect line and typed the answer.

use std::io::IsTerminal;

use anyhow::{bail, Result};

/// What a lowering is refused with when nobody can type the confirmation.
pub const NO_TERMINAL: &str =
    "Making this chat public needs a confirmation typed at a terminal; nothing was saved.";

/// What a person who declined is told.
pub const DECLINED: &str = "Nothing was saved: the chat was not made public.";

/// The second line under a lowering's effect line (the desktop's InfoTip).
pub const LOWERING_DETAIL: &str = biorouter::decision::save::BECOMES_PUBLIC_INFO;

/// The code the daemon answers a lowering with when the person's acceptance
/// is missing.
pub const NEEDS_CONFIRMATION: &str = "needsPublicDecisionConfirmation";

/// The daemon's and the save's words for a tier effect, in the terminal's:
/// a Baseten model by its alias (`joey-1`, not its slug), and "make it public
/// from History" as the command that does it here.
pub fn cli_words(text: &str) -> String {
    let mut out = text.to_string();
    for known in biorouter::providers::decision::catalog::BASETEN_DECISION_MODELS {
        let qualified = format!("baseten_decision/{}", known.slug);
        if out.contains(&qualified) {
            out = out.replace(&qualified, &format!("baseten_decision/{}", known.name));
        }
        out = out.replace(known.slug, known.name);
    }
    let out = out.replace(
        "make it public from History",
        "make it public with `biorouter session declassify`",
    );
    crate::commands::decide::terminal_text(&out)
}

/// The line a stored save's tier effect is said in at the terminal, or `None`
/// when the save does not touch the tier.
pub fn effect_line(saved: &biorouter::decision::save::CheckedSave) -> Option<String> {
    saved.effect_line.as_deref().map(cli_words)
}

/// Asks the person to confirm a lowering. A seam, so the flow can be tested
/// without a terminal.
pub trait ConfirmPublic: Send {
    /// Show `effect` (the effect line) and ask. `Ok(true)` confirmed,
    /// `Ok(false)` declined; `Err` is [`NO_TERMINAL`] when nobody can answer.
    fn confirm(&mut self, session_id: &str, effect: &str) -> Result<bool>;
}

/// The real one: a confirmation typed at a terminal.
pub struct TerminalConfirm {
    /// Whether a person can type here, sampled once ([`has_terminal`]).
    terminal: bool,
}

impl TerminalConfirm {
    /// The confirmation at this process's terminal, if it has one.
    pub fn here() -> Self {
        Self {
            terminal: has_terminal(),
        }
    }
}

/// Whether a person can type an answer here: stdin and stdout are both a
/// terminal.
pub fn has_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

impl ConfirmPublic for TerminalConfirm {
    fn confirm(&mut self, session_id: &str, effect: &str) -> Result<bool> {
        if !self.terminal {
            bail!("{NO_TERMINAL}");
        }
        let phrase = biorouter::privacy::declassify::confirmation_phrase(session_id);
        eprintln!("{effect}");
        eprintln!("{LOWERING_DETAIL}");
        let typed: String = cliclack::input(format!(
            "Type the last six characters of the chat id ({phrase}) to make it public, or leave \
             blank to cancel"
        ))
        .required(false)
        .interact()?;
        Ok(biorouter::privacy::declassify::confirmation_matches(
            session_id,
            Some(typed.as_str()).filter(|typed| !typed.trim().is_empty()),
        ))
    }
}

/// Ask `confirm` about a lowering whose effect line is `effect`, turning a
/// declined answer into [`DECLINED`].
pub fn require_confirmation(
    confirm: &mut dyn ConfirmPublic,
    session_id: &str,
    effect: &str,
) -> Result<()> {
    if confirm.confirm(session_id, effect)? {
        Ok(())
    } else {
        bail!("{DECLINED}")
    }
}

/// What one attempt at a save gave: it was stored, or it would make the chat
/// public and the person's acceptance is missing (the effect line to ask
/// with). Nothing is stored in the second case.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveAttempt<T> {
    Saved(T),
    NeedsConfirmation(String),
}

/// Run `save` without the person's acceptance; when it answers that the save
/// would make the chat public, ask `confirm` with its effect line and, only if
/// the person typed the answer, run it once more with the acceptance. The one
/// shape every terminal door that can lower a chat goes through, daemon and
/// local alike, so none of them can store a lowering unasked.
pub async fn with_public_confirmation<T, F, Fut>(
    confirm: &mut dyn ConfirmPublic,
    session_id: &str,
    mut save: F,
) -> Result<T>
where
    F: FnMut(bool) -> Fut,
    Fut: std::future::Future<Output = Result<SaveAttempt<T>>>,
{
    match save(false).await? {
        SaveAttempt::Saved(saved) => Ok(saved),
        SaveAttempt::NeedsConfirmation(effect) => {
            require_confirmation(confirm, session_id, &effect)?;
            match save(true).await? {
                SaveAttempt::Saved(saved) => Ok(saved),
                // The acceptance was sent and the save still asks: never loop.
                SaveAttempt::NeedsConfirmation(effect) => bail!(
                    "Nothing was saved: the save still asked for a confirmation after it was \
                     given ({effect})"
                ),
            }
        }
    }
}

/// The effect line a 409 `needsPublicDecisionConfirmation` carries: its
/// `message` (or `sentence`), else the body as text.
pub fn confirmation_question(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    parsed
        .as_ref()
        .and_then(|value| {
            ["message", "sentence", "effect", "error"]
                .iter()
                .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string())
}

/// Whether a daemon's answer is the 409 that asks for the person's acceptance
/// of a lowering, rather than any other conflict.
pub fn is_confirmation_request(code: u16, body: &str) -> bool {
    code == 409 && body.contains(NEEDS_CONFIRMATION)
}

/// Store `settings` for `session` at the terminal's door
/// (`decision::save::save_checked_by` with `SaveDoor::cli`): checked, with its
/// tier effect worked out first, and, when it would make the chat public,
/// asked about with `confirm` before it is stored with the person's
/// acceptance. Nothing is stored when the person declines or nobody can
/// answer ([`NO_TERMINAL`]).
pub async fn save_settings_at_terminal(
    service: &biorouter::decision::DecisionService,
    manager: &std::sync::Arc<biorouter::session::SessionManager>,
    session: &biorouter::decision::DecisionSession,
    settings: biorouter::decision::settings::DecisionSettings,
    confirm: &mut dyn ConfirmPublic,
) -> Result<biorouter::decision::save::CheckedSave> {
    use biorouter::decision::save::{
        save_checked_by, LoweringConsent, NeedsPublicDecisionConfirmation, SaveDoor,
    };
    let session_id = session.session_id.clone();
    with_public_confirmation(confirm, &session_id, |accepted| {
        let manager = std::sync::Arc::clone(manager);
        let session = session.clone();
        let settings = settings.clone();
        async move {
            let consent = if accepted {
                LoweringConsent::Confirmed
            } else {
                LoweringConsent::Unconfirmed
            };
            match save_checked_by(
                service,
                &manager,
                &session,
                settings,
                &SaveDoor::cli(consent),
            )
            .await
            {
                Ok(saved) => Ok(SaveAttempt::Saved(saved)),
                Err(error) => match error.downcast_ref::<NeedsPublicDecisionConfirmation>() {
                    Some(needs) => Ok(SaveAttempt::NeedsConfirmation(cli_words(&needs.sentence))),
                    None => Err(error),
                },
            }
        }
    })
    .await
}

/// A scripted answer, for tests.
#[cfg(test)]
pub struct Scripted {
    pub answer: Result<bool, &'static str>,
    pub asked: Vec<String>,
}

#[cfg(test)]
impl Scripted {
    pub fn yes() -> Self {
        Self {
            answer: Ok(true),
            asked: Vec::new(),
        }
    }
    pub fn no() -> Self {
        Self {
            answer: Ok(false),
            asked: Vec::new(),
        }
    }
    pub fn no_terminal() -> Self {
        Self {
            answer: Err(NO_TERMINAL),
            asked: Vec::new(),
        }
    }
}

#[cfg(test)]
impl ConfirmPublic for Scripted {
    fn confirm(&mut self, _session_id: &str, effect: &str) -> Result<bool> {
        self.asked.push(effect.to_string());
        self.answer
            .map_err(|sentence| anyhow::anyhow!("{sentence}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_says_a_baseten_model_by_its_alias_and_declassify_by_its_command() {
        assert_eq!(
            cli_words(
                "This chat will be public: decision-models/baseten-joey-2b-latest runs on \
                 Baseten."
            ),
            "This chat will be public: joey-1 runs on Baseten."
        );
        assert_eq!(
            cli_words("start a new chat with `biorouter session --decision-model baseten_decision/decision-models/baseten-mammoth-27b-latest`."),
            "start a new chat with `biorouter session --decision-model baseten_decision/mammoth-1`."
        );
        assert!(cli_words(
            "This chat becomes private at your next message. Public models can't be used in it \
             after that unless you make it public from History."
        )
        .ends_with("unless you make it public with `biorouter session declassify`."));
    }

    #[test]
    fn a_declined_or_unanswerable_confirmation_saves_nothing() {
        let mut no = Scripted::no();
        let err = require_confirmation(&mut no, "20261110_1", "This chat will be public.")
            .unwrap_err()
            .to_string();
        assert_eq!(err, DECLINED);
        assert_eq!(no.asked, vec!["This chat will be public.".to_string()]);

        let mut nobody = Scripted::no_terminal();
        let err = require_confirmation(&mut nobody, "20261110_1", "x")
            .unwrap_err()
            .to_string();
        assert_eq!(err, NO_TERMINAL);

        require_confirmation(&mut Scripted::yes(), "20261110_1", "x").unwrap();
    }

    /// Design §1.8 / S22: with no terminal on stdin and stdout, nobody can
    /// type the confirmation, so the lowering is refused before anything is
    /// asked or stored. There is no flag that answers it instead.
    #[test]
    fn without_a_terminal_the_confirmation_is_refused() {
        let err = TerminalConfirm { terminal: false }
            .confirm(
                "20261110_1",
                "This chat will be public: joey-1 runs on Baseten.",
            )
            .unwrap_err();
        assert_eq!(err.to_string(), NO_TERMINAL);
    }

    /// A save that would make the chat public is asked about first, and sent
    /// with the acceptance only after the person typed it; declined, the
    /// accepted save never runs.
    #[tokio::test]
    async fn a_lowering_is_sent_with_the_acceptance_only_after_the_person_confirms() {
        use std::sync::{Arc, Mutex};
        let sent = Arc::new(Mutex::new(Vec::<bool>::new()));
        let attempt = |sent: Arc<Mutex<Vec<bool>>>| {
            move |accepted: bool| {
                let sent = Arc::clone(&sent);
                async move {
                    sent.lock().unwrap().push(accepted);
                    Ok(if accepted {
                        SaveAttempt::Saved("stored")
                    } else {
                        SaveAttempt::NeedsConfirmation(
                            "This chat will be public: joey-1 runs on Baseten.".to_string(),
                        )
                    })
                }
            }
        };

        let mut yes = Scripted::yes();
        let saved = with_public_confirmation(&mut yes, "20261110_1", attempt(Arc::clone(&sent)))
            .await
            .unwrap();
        assert_eq!(saved, "stored");
        assert_eq!(*sent.lock().unwrap(), vec![false, true]);
        assert_eq!(
            yes.asked,
            vec!["This chat will be public: joey-1 runs on Baseten.".to_string()]
        );

        sent.lock().unwrap().clear();
        let err = with_public_confirmation(
            &mut Scripted::no_terminal(),
            "20261110_1",
            attempt(Arc::clone(&sent)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), NO_TERMINAL);
        assert_eq!(
            *sent.lock().unwrap(),
            vec![false],
            "nothing accepted was sent"
        );

        // A save that needs no confirmation is never asked about.
        let mut never = Scripted::no_terminal();
        let saved = with_public_confirmation(&mut never, "20261110_1", |_| async {
            Ok(SaveAttempt::Saved(1))
        })
        .await
        .unwrap();
        assert_eq!(saved, 1);
        assert!(never.asked.is_empty());
    }

    #[test]
    fn the_daemons_question_is_read_from_its_409() {
        let body = r#"{"code":"needsPublicDecisionConfirmation","message":"This chat will be public: joey-1 runs on Baseten."}"#;
        assert!(is_confirmation_request(409, body));
        assert!(!is_confirmation_request(409, "The phase is completed."));
        assert!(!is_confirmation_request(422, body));
        assert_eq!(
            confirmation_question(body),
            "This chat will be public: joey-1 runs on Baseten."
        );
        assert_eq!(confirmation_question("  plain  "), "plain");
    }
}
