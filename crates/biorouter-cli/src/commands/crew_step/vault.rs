//! A vault card in a terminal chat on the shared daemon: the passphrase is asked here with the
//! hidden prompt ([`crate::daemon_client::read_secret`], or [`crate::daemon_client::new_secret`]
//! twice for init) and posted to `/crew/credentials/{unlock,init}` with the person's proof,
//! through the chat's own [`crate::daemon_client::CrewClient`]. The value is never echoed,
//! printed or added to the chat; the card is never answered as done, because the call that
//! parked it reads the vault and takes it down itself. Skipping posts the card's dismissal.
//!
//! ⚠ **The person opens the prompt; a card never opens it by itself**, as on the desktop and as
//! a sign-in card here ([`super::sign_in`]). The card asks first ([`question`]), and only Enter
//! with nothing typed opens the hidden prompt. A card arrives when the model calls for it, so
//! opening at once read whatever the person was typing (often their next message) as a
//! passphrase and posted it to the vault route as a failed attempt.

use super::sign_in::{choice, Choice, ANSWER_BYTES, ASKS};
use super::StepSurface;
use crate::daemon_client::{new_secret, read_secret};
use anyhow::Result;
use biorouter::conversation::message::{CrewStepRequest, CrewVaultAction};
use serde_json::{json, Value};
use std::io::IsTerminal;
use zeroize::Zeroizing;

/// How many passphrases a card takes before it is left for Crew or `biorouter crew`.
const ATTEMPTS: usize = 3;

const VAULT_PASSPHRASE: &str = "Vault passphrase:";
const NEW_VAULT_PASSPHRASE: &str = "New vault passphrase:";
const VAULT_PASSPHRASE_AGAIN: &str = "Type the same vault passphrase again:";
const VAULT_PASSPHRASES_DIFFER: &str = "The two passphrases don't match. The vault was not set up.";

/// Said before anything is asked: what the card needs, and where the passphrase goes.
fn opening(action: CrewVaultAction) -> &'static str {
    match action {
        CrewVaultAction::Unlock => "This chat needs Crew's vault on this computer, and it is locked. Its passphrase goes only to Crew on this computer, never to the chat.",
        CrewVaultAction::Init => "This chat needs a vault on this computer for Crew's keys. You choose its passphrase and type it twice. It goes only to Crew on this computer, never to the chat.",
    }
}

/// Asked before the hidden prompt opens.
fn question(action: CrewVaultAction) -> &'static str {
    match action {
        CrewVaultAction::Unlock => "Unlock Crew's vault now? Press Enter to open the hidden passphrase prompt, or type n to skip: ",
        CrewVaultAction::Init => "Set up Crew's vault now? Press Enter to open the hidden passphrase prompt, or type n to skip: ",
    }
}

/// Said when a line answers neither way. What was typed is not repeated, posted or sent
/// anywhere: it may be a passphrase typed too early.
const ANSWER_AGAIN: &str = "That opened nothing and went nowhere. Press Enter with nothing typed to open the passphrase prompt, or type n to skip.";

/// Said once the hidden prompt is open.
const AT_THE_PROMPT: &str =
    "Type the passphrase at the hidden prompt. Press Enter with nothing typed to skip.";

/// Said once the daemon took the passphrase.
fn done(action: CrewVaultAction) -> &'static str {
    match action {
        CrewVaultAction::Unlock => "Crew's vault is unlocked. The chat carries on.",
        CrewVaultAction::Init => "Crew's vault is set up and unlocked. The chat carries on.",
    }
}

/// Said when the card is left: skipped, or refused [`ATTEMPTS`] times.
fn left(action: CrewVaultAction) -> &'static str {
    match action {
        CrewVaultAction::Unlock => "Crew's vault is still locked. You can unlock it in Crew, under Keys, or with biorouter crew credentials unlock.",
        CrewVaultAction::Init => "Crew's vault is still not set up. You can set it up in Crew, under Keys, or with biorouter crew credentials init.",
    }
}

/// Where to unlock or set up the vault when this terminal can't ask: the desktop app, or
/// `biorouter crew credentials`. The card stays up meanwhile, so the chat carries on once the
/// vault is open.
pub(crate) fn fallback(step: &CrewStepRequest) -> Option<String> {
    let CrewStepRequest::Vault { action } = step else {
        return None;
    };
    Some(match action {
        CrewVaultAction::Unlock => "Unlock Crew's vault in the Biorouter desktop app, under Keys in Crew, or run biorouter crew credentials unlock in a terminal. Type the passphrase only there, never in the chat.",
        CrewVaultAction::Init => "Set up Crew's vault in the Biorouter desktop app, under Keys in Crew, or run biorouter crew credentials init in a terminal. Type the passphrase only there, never in the chat.",
    }.to_owned())
}

/// What a vault card needs from this terminal and the daemon, so a test can stand in for both.
trait Desk {
    /// One line for the person on the chat's notice channel. Never a passphrase.
    fn say(&self, line: &str) -> Result<()>;
    /// Whether a hidden prompt can be shown here.
    fn can_prompt(&self) -> bool;
    /// One line the person types after `question`, `Ok(None)` for Esc or Ctrl-C. Zeroized,
    /// because a person may type a passphrase here before the prompt opens.
    async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>>;
    /// The passphrase for `action`, from the hidden prompt (twice for init). `Ok(None)` is a
    /// skip: nothing typed. An `Err` is said, and the passphrase asked again.
    async fn passphrase(&self, action: CrewVaultAction) -> Result<Option<Zeroizing<String>>>;
    /// Post `body` to the vault route for `action`, with the person's proof.
    async fn post(&self, action: CrewVaultAction, body: Value) -> Result<Value>;
    /// Take the card down as not done.
    async fn dismiss(&self) -> Result<()>;
}

impl Desk for StepSurface<'_> {
    fn say(&self, line: &str) -> Result<()> {
        StepSurface::say(self, line)
    }

    fn can_prompt(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }

    async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>> {
        crate::commands::shared_conversation::terminal_input(question, false, ANSWER_BYTES).await
    }

    async fn passphrase(&self, action: CrewVaultAction) -> Result<Option<Zeroizing<String>>> {
        let typed = match action {
            CrewVaultAction::Unlock => read_secret(VAULT_PASSPHRASE, false).await,
            CrewVaultAction::Init => {
                new_secret(
                    [
                        NEW_VAULT_PASSPHRASE,
                        VAULT_PASSPHRASE_AGAIN,
                        VAULT_PASSPHRASES_DIFFER,
                    ],
                    false,
                    read_secret,
                )
                .await
            }
        };
        match typed {
            Ok(passphrase) => Ok(Some(passphrase)),
            // Two that differ are asked again; anything else (nothing typed, Ctrl-C) skips.
            Err(error) if error.to_string() == VAULT_PASSPHRASES_DIFFER => Err(error),
            Err(_) => Ok(None),
        }
    }

    async fn post(&self, action: CrewVaultAction, body: Value) -> Result<Value> {
        let path = match action {
            CrewVaultAction::Unlock => "/crew/credentials/unlock",
            CrewVaultAction::Init => "/crew/credentials/init",
        };
        self.client.request("POST", path, Some(body)).await
    }

    async fn dismiss(&self) -> Result<()> {
        self.client
            .request(
                "POST",
                "/action-required/crew-step",
                Some(json!({"id": self.card_id, "session_id": self.session_id, "outcome": "dismiss"})),
            )
            .await
            .map(|_| ())
    }
}

/// Take a vault card: ask for the passphrase and post it, up to [`ATTEMPTS`] times.
pub(crate) async fn take(surface: &StepSurface<'_>, step: &CrewStepRequest) -> Result<()> {
    let CrewStepRequest::Vault { action } = step else {
        return Ok(());
    };
    take_at(surface, *action).await
}

async fn take_at(desk: &impl Desk, action: CrewVaultAction) -> Result<()> {
    if !desk.can_prompt() {
        // The card stays up: unlocking in Crew or another terminal lets the chat carry on.
        if let Some(line) = fallback(&CrewStepRequest::Vault { action }) {
            desk.say(&line)?;
        }
        return Ok(());
    }
    desk.say(opening(action))?;
    if !person_opens(desk, action).await? {
        return leave(desk, action).await;
    }
    desk.say(AT_THE_PROMPT)?;
    for _ in 0..ATTEMPTS {
        let passphrase = match desk.passphrase(action).await {
            Ok(Some(passphrase)) => passphrase,
            Ok(None) => return leave(desk, action).await,
            Err(error) => {
                desk.say(&format!("{error:#}"))?;
                continue;
            }
        };
        let refused = match desk
            .post(action, json!({"passphrase": passphrase.as_str()}))
            .await
        {
            Ok(_) => return desk.say(done(action)),
            Err(error) => format!("{error:#}"),
        };
        // The daemon never repeats a passphrase; were its words ever to, they are not shown.
        if refused.contains(passphrase.as_str()) {
            desk.say("Crew didn't take that passphrase.")?;
        } else {
            desk.say(&format!("Crew didn't take that passphrase: {refused}"))?;
        }
    }
    leave(desk, action).await
}

/// Whether the person chose to open the hidden prompt: [`question`] asked up to [`ASKS`]
/// times, and only Enter with nothing typed says yes. A terminal that could not be read says
/// no. A line that says neither (a message, a passphrase typed too early) is never used.
async fn person_opens(desk: &impl Desk, action: CrewVaultAction) -> Result<bool> {
    for asked in 1..=ASKS {
        let typed = desk.ask(question(action)).await.unwrap_or(None);
        match choice(typed.as_ref().map(|typed| typed.as_str())) {
            Choice::Open => return Ok(true),
            Choice::Skip => return Ok(false),
            Choice::Again if asked < ASKS => desk.say(ANSWER_AGAIN)?,
            Choice::Again => {}
        }
    }
    Ok(false)
}

/// Leave the card: say where to finish, and take it down so the chat goes on.
async fn leave(desk: &impl Desk, action: CrewVaultAction) -> Result<()> {
    desk.say(left(action))?;
    if let Err(error) = desk.dismiss().await {
        desk.say(&format!("The card couldn't be closed: {error:#}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const CANARY: &str = "CANARY-7f3a vault passphrase";

    /// What the person does at one hidden prompt.
    #[derive(Clone, Copy)]
    enum Typed {
        Text(&'static str),
        Differ,
        Nothing,
    }

    /// A terminal and a daemon in memory: the prompts answered in order, the daemon's answers
    /// in order, and everything said, posted and dismissed.
    struct Fake {
        terminal: bool,
        typed: RefCell<Vec<Typed>>,
        /// The lines typed at the question, in order; Enter once none are left. `None` is Esc.
        answered: RefCell<Vec<Option<&'static str>>>,
        questions: RefCell<Vec<String>>,
        answers: RefCell<Vec<Result<Value, &'static str>>>,
        said: RefCell<Vec<String>>,
        posted: RefCell<Vec<(CrewVaultAction, Value)>>,
        dismissed: RefCell<usize>,
    }

    impl Fake {
        fn new(typed: &[Typed], answers: Vec<Result<Value, &'static str>>) -> Self {
            Self {
                terminal: true,
                typed: RefCell::new(typed.iter().rev().copied().collect()),
                answered: RefCell::default(),
                questions: RefCell::default(),
                answers: RefCell::new(answers.into_iter().rev().collect()),
                said: RefCell::default(),
                posted: RefCell::default(),
                dismissed: RefCell::default(),
            }
        }

        fn answering(mut self, lines: &[Option<&'static str>]) -> Self {
            self.answered = RefCell::new(lines.iter().rev().copied().collect());
            self
        }

        fn said(&self) -> String {
            self.said.borrow().join("\n")
        }
    }

    impl Desk for Fake {
        fn say(&self, line: &str) -> Result<()> {
            self.said.borrow_mut().push(line.to_owned());
            Ok(())
        }

        fn can_prompt(&self) -> bool {
            self.terminal
        }

        async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>> {
            self.questions.borrow_mut().push(question.to_owned());
            Ok(self
                .answered
                .borrow_mut()
                .pop()
                .unwrap_or(Some(""))
                .map(|line| Zeroizing::new(line.to_owned())))
        }

        async fn passphrase(&self, _action: CrewVaultAction) -> Result<Option<Zeroizing<String>>> {
            match self
                .typed
                .borrow_mut()
                .pop()
                .expect("a prompt was answered")
            {
                Typed::Text(text) => Ok(Some(Zeroizing::new(text.to_owned()))),
                Typed::Differ => anyhow::bail!(VAULT_PASSPHRASES_DIFFER),
                Typed::Nothing => Ok(None),
            }
        }

        async fn post(&self, action: CrewVaultAction, body: Value) -> Result<Value> {
            self.posted.borrow_mut().push((action, body));
            match self.answers.borrow_mut().pop().expect("an answer") {
                Ok(answer) => Ok(answer),
                Err(refusal) => Err(anyhow::anyhow!(refusal)),
            }
        }

        async fn dismiss(&self) -> Result<()> {
            *self.dismissed.borrow_mut() += 1;
            Ok(())
        }
    }

    fn unlocked() -> Value {
        json!({"backend": "encrypted_vault", "initialized": true, "locked": false})
    }

    /// The passphrase goes only into the POST body; nothing said carries it, a refusal is said
    /// and asked again, and the card is not dismissed once the daemon took it.
    #[tokio::test]
    async fn the_passphrase_goes_only_to_the_vault_route() {
        let fake = Fake::new(
            &[Typed::Text(CANARY), Typed::Text("the real one")],
            vec![
                Err("Crew vault authentication failed: incorrect passphrase, profile mismatch, or damaged vault"),
                Ok(unlocked()),
            ],
        );
        take_at(&fake, CrewVaultAction::Unlock).await.unwrap();
        let posted = fake.posted.borrow();
        assert_eq!(posted.len(), 2);
        assert_eq!(
            posted[0],
            (CrewVaultAction::Unlock, json!({"passphrase": CANARY}))
        );
        assert_eq!(posted[1].1, json!({"passphrase": "the real one"}));
        let said = fake.said();
        assert!(!said.contains(CANARY), "{said}");
        assert!(!said.contains("the real one"), "{said}");
        assert!(said.contains("incorrect passphrase"), "{said}");
        assert!(said.ends_with(done(CrewVaultAction::Unlock)), "{said}");
        assert_eq!(*fake.dismissed.borrow(), 0);
    }

    /// WI-17 (e): the card asks before any prompt opens, so a line the person had already typed
    /// (their next message, or a passphrase typed too early) answers the question, is never
    /// read as a passphrase and never posted: the prompt opens only on Enter with nothing typed.
    #[tokio::test]
    async fn the_prompt_opens_only_when_the_person_presses_enter() {
        let early = "CANARY-7f3a my next chat message";
        let fake = Fake::new(&[Typed::Text("the real one")], vec![Ok(unlocked())])
            .answering(&[Some(early)]);
        take_at(&fake, CrewVaultAction::Unlock).await.unwrap();
        assert_eq!(
            *fake.questions.borrow(),
            [question(CrewVaultAction::Unlock); 2],
            "asked again after a line that said neither"
        );
        assert_eq!(
            *fake.posted.borrow(),
            [(
                CrewVaultAction::Unlock,
                json!({"passphrase": "the real one"})
            )],
            "only the passphrase typed at the open prompt is posted"
        );
        let said = fake.said();
        assert!(!said.contains("CANARY-7f3a"), "{said}");
        assert!(said.contains(ANSWER_AGAIN), "{said}");
        assert!(said.ends_with(done(CrewVaultAction::Unlock)), "{said}");

        // `n` (or Esc) skips: no prompt opens, nothing is posted, and the card is dismissed.
        for skip in [Some("n"), None] {
            let fake = Fake::new(&[], vec![]).answering(&[skip]);
            take_at(&fake, CrewVaultAction::Init).await.unwrap();
            assert!(fake.posted.borrow().is_empty(), "{skip:?}");
            assert_eq!(*fake.dismissed.borrow(), 1, "{skip:?}");
            assert!(
                fake.said().ends_with(left(CrewVaultAction::Init)),
                "{skip:?}"
            );
        }

        // Three lines that say neither skip too, and none of them reaches the vault route.
        let fake = Fake::new(&[], vec![]).answering(&[Some(early), Some(early), Some(early)]);
        take_at(&fake, CrewVaultAction::Unlock).await.unwrap();
        assert!(fake.posted.borrow().is_empty());
        assert_eq!(*fake.dismissed.borrow(), 1);
        assert!(!fake.said().contains("CANARY-7f3a"));
    }

    /// A refusal that ever repeated the passphrase is not shown.
    #[tokio::test]
    async fn a_refusal_that_repeats_the_passphrase_is_not_shown() {
        let fake = Fake::new(
            &[Typed::Text(CANARY), Typed::Nothing],
            vec![Err("rejected CANARY-7f3a vault passphrase")],
        );
        take_at(&fake, CrewVaultAction::Unlock).await.unwrap();
        let said = fake.said();
        assert!(!said.contains(CANARY), "{said}");
        assert!(said.contains("Crew didn't take that passphrase."), "{said}");
    }

    /// Nothing typed leaves the card: nothing is posted, and the card is dismissed.
    #[tokio::test]
    async fn nothing_typed_posts_nothing_and_dismisses_the_card() {
        let fake = Fake::new(&[Typed::Nothing], vec![]);
        take_at(&fake, CrewVaultAction::Init).await.unwrap();
        assert!(fake.posted.borrow().is_empty());
        assert_eq!(*fake.dismissed.borrow(), 1);
        assert!(fake.said().ends_with(left(CrewVaultAction::Init)));
    }

    /// Two passphrases that differ are said and asked again; the init route gets the third.
    #[tokio::test]
    async fn init_asks_again_when_the_two_differ() {
        let fake = Fake::new(&[Typed::Differ, Typed::Text(CANARY)], vec![Ok(unlocked())]);
        take_at(&fake, CrewVaultAction::Init).await.unwrap();
        assert!(fake.said().contains(VAULT_PASSPHRASES_DIFFER));
        assert_eq!(
            *fake.posted.borrow(),
            [(CrewVaultAction::Init, json!({"passphrase": CANARY}))]
        );
        assert!(!fake.said().contains(CANARY));
    }

    /// Three refusals leave the card for Crew or `biorouter crew`.
    #[tokio::test]
    async fn three_refusals_leave_the_card() {
        let fake = Fake::new(
            &[Typed::Text("one"), Typed::Text("two"), Typed::Text("three")],
            vec![Err("no"), Err("no"), Err("no")],
        );
        take_at(&fake, CrewVaultAction::Unlock).await.unwrap();
        assert_eq!(fake.posted.borrow().len(), ATTEMPTS);
        assert_eq!(*fake.dismissed.borrow(), 1);
        assert!(fake.said().ends_with(left(CrewVaultAction::Unlock)));
    }

    /// With no terminal to prompt in, it says where to do it and leaves the card up, so the
    /// chat carries on once the vault is open; nothing is asked or posted.
    #[tokio::test]
    async fn without_a_terminal_it_says_where_and_leaves_the_card_up() {
        let mut fake = Fake::new(&[], vec![]);
        fake.terminal = false;
        take_at(&fake, CrewVaultAction::Unlock).await.unwrap();
        assert_eq!(
            fake.said(),
            fallback(&CrewStepRequest::Vault {
                action: CrewVaultAction::Unlock
            })
            .unwrap()
        );
        assert!(fake.posted.borrow().is_empty());
        assert_eq!(*fake.dismissed.borrow(), 0);
    }

    #[test]
    fn the_fallback_names_the_desktop_and_the_command_for_the_action() {
        for (action, command) in [
            (CrewVaultAction::Unlock, "biorouter crew credentials unlock"),
            (CrewVaultAction::Init, "biorouter crew credentials init"),
        ] {
            let text = fallback(&CrewStepRequest::Vault { action }).unwrap();
            assert!(text.contains(command), "{text}");
            assert!(text.contains("desktop app"), "{text}");
            assert!(text.contains("never in the chat"), "{text}");
            assert!(left(action).contains(command), "{action:?}");
        }
    }

    #[test]
    fn the_words_follow_the_copy_rules() {
        for action in [CrewVaultAction::Unlock, CrewVaultAction::Init] {
            for text in [
                opening(action),
                question(action),
                ANSWER_AGAIN,
                AT_THE_PROMPT,
                done(action),
                left(action),
            ] {
                assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
                assert!(!text.to_lowercase().contains("device"), "{text}");
            }
        }
    }
}
