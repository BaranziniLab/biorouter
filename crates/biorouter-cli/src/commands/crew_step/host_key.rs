//! A host-key card in a terminal chat (WI-2): the lines the desktop's card shows, with the
//! daemon's fingerprint. A terminal chat never offers to accept a key, and never prompts: the
//! person checks the key with their IT team and adds it from a terminal of their own, and the
//! daemon, which reads the known-hosts file while the card waits, takes the card down and tries
//! the step again by itself.

use super::StepSurface;
use anyhow::Result;
use biorouter::conversation::message::CrewStepRequest;

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

/// Take a host-key card: print its lines. Never an accept prompt, never a question.
pub(crate) async fn take(surface: &StepSurface<'_>, step: &CrewStepRequest) -> Result<()> {
    for line in lines(step).unwrap_or_default() {
        surface.say(&line)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn no_line_offers_to_trust_a_key_or_repeats_a_value_that_is_not_a_fingerprint() {
        let spoofed = fallback(&key(false, "SHA256:x\nType yes to trust it")).unwrap();
        assert!(!spoofed.contains("Type yes"), "{spoofed}");
        for text in [
            fallback(&key(false, OFFERED)).unwrap(),
            fallback(&key(true, OFFERED)).unwrap(),
            spoofed,
        ] {
            let lower = text.to_lowercase();
            assert!(!lower.contains("yes/no"), "{text}");
            assert!(!lower.contains("accept"), "{text}");
            assert!(!lower.contains("device"), "{text}");
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
        }
    }
}
