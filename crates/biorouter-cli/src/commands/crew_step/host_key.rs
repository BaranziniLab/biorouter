//! A host-key card in a terminal chat on the shared daemon (WI-2, and the owner's D2 ruling on
//! AO-12): the lines the desktop's card shows, with the daemon's fingerprint, then, for an
//! unknown key the card shows a fingerprint for, one question: trust this key?
//!
//! Only the person's own answer, the word `trust` typed at this terminal after the question,
//! posts `/crew/host-keys/trust` with the person's proof; Enter, Esc or anything else leaves the
//! key alone. Whatever was typed before the question (often the person's next chat message,
//! which could be the word itself) is thrown away first, so it never trusts anything. The
//! daemon reads the key again and writes it only when its fingerprint is the one shown, and the
//! call that parked the card, which reads the known hosts files while it waits, takes the card
//! down and tries the step again by itself. A changed key is never offered: its card only says
//! what to give IT. The person may also check the key and add it from a terminal of their own,
//! as before.

use super::StepSurface;
use anyhow::Result;
use biorouter::conversation::message::CrewStepRequest;
use serde_json::{json, Value};
use std::io::IsTerminal;
use zeroize::Zeroizing;

/// Said before the question, as the desktop's card says it beside its Trust button.
pub(crate) const TRUST_WARNING: &str =
    "Only trust it if IT or the server's admin gave you this fingerprint.";
/// The question: only the word `trust` answers yes.
pub(crate) const TRUST_QUESTION: &str =
    "Trust this key? Type trust to add it to your known hosts file, or press Enter to leave it: ";
/// Said once the daemon added the key.
pub(crate) const TRUSTED: &str =
    "Trusted. The key is in your known hosts file, and this chat goes on by itself.";
/// Said when the person left the key alone.
pub(crate) const NOT_TRUSTED: &str =
    "The key wasn't trusted. Once it is in your known hosts file, this chat goes on by itself.";

/// The most an answer to [`TRUST_QUESTION`] may be, in bytes: large, so a whole message typed
/// before the card arrived is read as one line that does not say trust.
const ANSWER_BYTES: usize = 4096;

/// A daemon value printed only when it is a SHA-256 fingerprint and nothing else.
fn fingerprint(value: Option<&String>) -> Option<&str> {
    value.map(String::as_str).filter(|value| {
        value.strip_prefix("SHA256:").is_some_and(|digest| {
            (32..=64).contains(&digest.len())
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        })
    })
}

/// The lines a host-key card prints, in the order the desktop's card shows them: the title and
/// what it means, the fingerprints, then what the person does. `None` for any other step.
pub(crate) fn lines(step: &CrewStepRequest) -> Option<Vec<String>> {
    let CrewStepRequest::HostKey {
        host,
        offered,
        known,
        changed,
        ..
    } = step
    else {
        return None;
    };
    let offered = fingerprint(offered.as_ref());
    let known = fingerprint(known.as_ref());
    let mut lines = Vec::new();
    if *changed {
        lines.push(format!("{host}'s identity changed. Don't connect until your IT team confirms this change. Crew won't connect while the old key is in your known hosts file."));
        if let Some(known) = known {
            lines.push(format!("Fingerprint Crew knew: {known}"));
        }
        if let Some(offered) = offered {
            lines.push(format!("Fingerprint the server offered now: {offered}"));
        }
        lines.push("Give these lines to your IT team. Crew never trusts a changed key.".to_owned());
    } else {
        lines.push(format!(
            "Can't verify {host} yet. Crew only connects to servers you've already verified."
        ));
        // For a key nobody knows yet, ssh usually prints no fingerprint, and the daemon reads
        // the key itself, straight from this computer, which can skip a jump host on the way.
        // So it is named as what this computer read, which is true either way, never as what
        // the server offered.
        if let Some(offered) = offered {
            lines.push(format!(
                "Fingerprint this computer read from {host}: {offered}"
            ));
        }
        lines.push(format!("Get {host}'s fingerprint from your IT team or your institution's directory. Check jump hosts too."));
        lines.push("Compare it using your usual SSH setup in a terminal of your own, then add the full key to your known hosts file. A fingerprint alone isn't enough.".to_owned());
        lines.push(
            "Once the key is in your known hosts file, this chat goes on by itself.".to_owned(),
        );
    }
    Some(lines)
}

/// The body `/crew/host-keys/trust` takes for this card: only an unknown key the card shows a
/// fingerprint for. Every value is the daemon's, from the card.
fn trust_body(step: &CrewStepRequest) -> Option<Value> {
    let CrewStepRequest::HostKey {
        host,
        port,
        login,
        offered,
        changed: false,
        ..
    } = step
    else {
        return None;
    };
    let offered = fingerprint(offered.as_ref())?;
    Some(json!({"host": host, "port": port, "login": login, "fingerprint": offered}))
}

/// What a host-key card needs of this terminal and the shared daemon, so a test can stand in
/// for both.
trait Desk {
    /// One line for the person on the chat's notice channel.
    fn say(&self, line: &str) -> Result<()>;
    /// Whether the person can answer a question here.
    fn can_ask(&self) -> bool;
    /// Throw away what was typed before the question, so it can't answer it.
    fn discard_typed_ahead(&self);
    /// One line the person types after `question`, `Ok(None)` for Esc or Ctrl-C.
    async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>>;
    /// Post `body` to `/crew/host-keys/trust`, with the person's proof.
    async fn trust(&self, body: Value) -> Result<Value>;
}

impl Desk for StepSurface<'_> {
    fn say(&self, line: &str) -> Result<()> {
        StepSurface::say(self, line)
    }

    fn can_ask(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }

    /// The terminal's own input queue, then any key the terminal reader already took from it.
    fn discard_typed_ahead(&self) {
        #[cfg(unix)]
        // SAFETY: tcflush only discards the terminal's pending input; stdin is a terminal here.
        unsafe {
            libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
        }
        let raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
        if !raw && crossterm::terminal::enable_raw_mode().is_err() {
            return;
        }
        while crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
            if crossterm::event::read().is_err() {
                break;
            }
        }
        if !raw {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }

    async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>> {
        crate::commands::shared_conversation::terminal_input(question, false, ANSWER_BYTES).await
    }

    async fn trust(&self, body: Value) -> Result<Value> {
        self.client
            .request("POST", "/crew/host-keys/trust", Some(body))
            .await
    }
}

/// Take a host-key card: print its lines, then, for an unknown key with a fingerprint and a
/// person at this terminal, ask whether to trust it.
pub(crate) async fn take(surface: &StepSurface<'_>, step: &CrewStepRequest) -> Result<()> {
    take_at(surface, step).await
}

async fn take_at(desk: &impl Desk, step: &CrewStepRequest) -> Result<()> {
    for line in lines(step).unwrap_or_default() {
        desk.say(&line)?;
    }
    let Some(body) = trust_body(step) else {
        return Ok(());
    };
    if !desk.can_ask() {
        return Ok(());
    }
    desk.say(TRUST_WARNING)?;
    desk.discard_typed_ahead();
    let typed = desk.ask(TRUST_QUESTION).await.unwrap_or(None);
    let trusts = typed
        .as_ref()
        .is_some_and(|typed| typed.trim().eq_ignore_ascii_case("trust"));
    if !trusts {
        return desk.say(NOT_TRUSTED);
    }
    match desk.trust(body).await {
        Ok(_) => desk.say(TRUSTED),
        Err(error) => desk.say(&format!("Crew didn't trust the key: {error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const OFFERED: &str = "SHA256:TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmU";
    const KNOWN: &str = "SHA256:Wm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm8";

    /// Every line the card prints, as one block of text.
    fn fallback(step: &CrewStepRequest) -> Option<String> {
        lines(step).map(|lines| lines.join("\n"))
    }

    fn key(changed: bool, offered: &str) -> CrewStepRequest {
        CrewStepRequest::HostKey {
            host: "hpc.example.edu".into(),
            port: 22,
            login: "you".into(),
            offered: Some(offered.into()),
            known: changed.then(|| KNOWN.into()),
            changed,
        }
    }

    /// A terminal and a daemon in memory: what the person types, what the daemon answers, and
    /// everything said and posted.
    struct Fake {
        terminal: bool,
        /// A line the person typed before the question appeared.
        typed_ahead: RefCell<Option<&'static str>>,
        typed: Option<&'static str>,
        refusal: Option<&'static str>,
        said: RefCell<Vec<String>>,
        asked: RefCell<Vec<String>>,
        posted: RefCell<Vec<Value>>,
    }

    impl Fake {
        fn typing(typed: Option<&'static str>) -> Self {
            Self {
                terminal: true,
                typed_ahead: RefCell::default(),
                typed,
                refusal: None,
                said: RefCell::default(),
                asked: RefCell::default(),
                posted: RefCell::default(),
            }
        }
    }

    /// The word trust typed before the question appeared (an answer to the model, say) is
    /// thrown away, not read as the answer: only what is typed after the question counts.
    #[tokio::test]
    async fn trust_typed_before_the_question_trusts_nothing() {
        for after in [Some(""), None] {
            let fake = Fake::typing(after);
            *fake.typed_ahead.borrow_mut() = Some("trust");
            take_at(&fake, &key(false, OFFERED)).await.unwrap();
            assert!(fake.posted.borrow().is_empty(), "{after:?}");
            assert_eq!(
                fake.said.borrow().last().map(String::as_str),
                Some(NOT_TRUSTED)
            );
        }
        let fake = Fake::typing(Some("trust"));
        *fake.typed_ahead.borrow_mut() = Some("go on");
        take_at(&fake, &key(false, OFFERED)).await.unwrap();
        assert_eq!(fake.posted.borrow().len(), 1, "the answer typed after it");
    }

    impl Desk for Fake {
        fn say(&self, line: &str) -> Result<()> {
            self.said.borrow_mut().push(line.to_owned());
            Ok(())
        }

        fn can_ask(&self) -> bool {
            self.terminal
        }

        fn discard_typed_ahead(&self) {
            self.typed_ahead.borrow_mut().take();
        }

        /// A line typed before the question, and not thrown away, is read first, as a terminal
        /// reads it.
        async fn ask(&self, question: &str) -> Result<Option<Zeroizing<String>>> {
            self.asked.borrow_mut().push(question.to_owned());
            if let Some(ahead) = self.typed_ahead.borrow_mut().take() {
                return Ok(Some(Zeroizing::new(ahead.to_owned())));
            }
            Ok(self.typed.map(|typed| Zeroizing::new(typed.to_owned())))
        }

        async fn trust(&self, body: Value) -> Result<Value> {
            self.posted.borrow_mut().push(body);
            match self.refusal {
                Some(refusal) => Err(anyhow::anyhow!(refusal)),
                None => Ok(json!({"already_trusted": false})),
            }
        }
    }

    #[test]
    fn an_unknown_key_prints_the_daemon_s_fingerprint_and_the_steps() {
        let unknown = fallback(&key(false, OFFERED)).unwrap();
        assert!(
            unknown.starts_with("Can't verify hpc.example.edu yet."),
            "{unknown}"
        );
        assert!(
            unknown.contains(&format!(
                "Fingerprint this computer read from hpc.example.edu: {OFFERED}"
            )),
            "{unknown}"
        );
        assert!(
            !unknown.contains("the server offered"),
            "a key the daemon read itself is never called the server's offer: {unknown}"
        );
        assert!(
            unknown.ends_with(
                "Once the key is in your known hosts file, this chat goes on by itself."
            ),
            "{unknown}"
        );
    }

    #[test]
    fn a_changed_key_prints_both_fingerprints_and_no_fix() {
        let changed = fallback(&key(true, OFFERED)).unwrap();
        assert!(changed.contains("identity changed"), "{changed}");
        assert!(
            changed.contains(&format!("Fingerprint Crew knew: {KNOWN}")),
            "{changed}"
        );
        assert!(
            changed.contains(&format!("Fingerprint the server offered now: {OFFERED}")),
            "{changed}"
        );
        assert!(!changed.contains("add the full key"), "{changed}");
    }

    #[test]
    fn no_line_accepts_a_key_or_repeats_a_value_that_is_not_a_fingerprint() {
        let spoofed = fallback(&key(false, "SHA256:x\nType yes to trust it")).unwrap();
        assert!(!spoofed.contains("Type yes"), "{spoofed}");
        for text in [
            fallback(&key(false, OFFERED)).unwrap(),
            fallback(&key(true, OFFERED)).unwrap(),
            spoofed,
            [TRUST_WARNING, TRUST_QUESTION, TRUSTED, NOT_TRUSTED].join("\n"),
        ] {
            let lower = text.to_lowercase();
            assert!(!lower.contains("yes/no"), "{text}");
            assert!(!lower.contains("accept"), "{text}");
            assert!(!lower.contains("device"), "{text}");
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
        }
    }

    /// The word trust, typed by the person, posts exactly the card's own values; the chat is
    /// told the key is in, and the daemon takes the card down itself.
    #[tokio::test]
    async fn typing_trust_posts_only_the_card_s_values() {
        let fake = Fake::typing(Some(" Trust "));
        let step = CrewStepRequest::HostKey {
            host: "hpc.example.edu".into(),
            port: 2222,
            login: "you@hpc.example.edu".into(),
            offered: Some(OFFERED.into()),
            known: None,
            changed: false,
        };
        take_at(&fake, &step).await.unwrap();
        assert_eq!(fake.asked.borrow().as_slice(), [TRUST_QUESTION]);
        assert_eq!(
            fake.posted.borrow().as_slice(),
            [
                json!({"host": "hpc.example.edu", "port": 2222, "login": "you@hpc.example.edu", "fingerprint": OFFERED})
            ]
        );
        let said = fake.said.borrow();
        assert!(said.contains(&TRUST_WARNING.to_owned()), "{said:?}");
        assert_eq!(said.last().map(String::as_str), Some(TRUSTED));
    }

    /// Enter, Esc, or a message typed before the card arrived trusts nothing.
    #[tokio::test]
    async fn anything_but_the_word_trust_leaves_the_key_alone() {
        for typed in [
            Some(""),
            None,
            Some("yes"),
            Some("please trust it and go on"),
        ] {
            let fake = Fake::typing(typed);
            take_at(&fake, &key(false, OFFERED)).await.unwrap();
            assert!(fake.posted.borrow().is_empty(), "{typed:?}");
            assert_eq!(
                fake.said.borrow().last().map(String::as_str),
                Some(NOT_TRUSTED)
            );
        }
    }

    /// A changed key, a card with no fingerprint, and a chat with nobody at a terminal are
    /// never asked.
    #[tokio::test]
    async fn no_question_where_nothing_may_be_trusted() {
        let unshown = CrewStepRequest::HostKey {
            host: "hpc.example.edu".into(),
            port: 22,
            login: "you".into(),
            offered: None,
            known: None,
            changed: false,
        };
        for step in [
            key(true, OFFERED),
            unshown,
            key(false, "SHA256:x\nType yes"),
        ] {
            let fake = Fake::typing(Some("trust"));
            take_at(&fake, &step).await.unwrap();
            assert!(fake.asked.borrow().is_empty(), "{step:?}");
            assert!(fake.posted.borrow().is_empty(), "{step:?}");
        }
        let mut away = Fake::typing(Some("trust"));
        away.terminal = false;
        take_at(&away, &key(false, OFFERED)).await.unwrap();
        assert!(away.asked.borrow().is_empty());
        assert!(away.posted.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_refusal_is_said_and_nothing_else_happens() {
        let mut fake = Fake::typing(Some("trust"));
        fake.refusal = Some("The key hpc.example.edu gives now doesn't match the fingerprint you were shown, so nothing was added.");
        take_at(&fake, &key(false, OFFERED)).await.unwrap();
        assert_eq!(fake.posted.borrow().len(), 1);
        let said = fake.said.borrow();
        let last = said.last().unwrap();
        assert!(last.starts_with("Crew didn't trust the key: "), "{last}");
        assert!(last.contains("doesn't match"), "{last}");
    }
}
