//! The macOS helper that types a finished dictation into the app the person was using when
//! the dictation started (DD-97 in `docs/history/local-dictation-2026-11/implementation-plan.md`).
//!
//! The desktop app starts this program as a child while fn in other apps or the global dictation
//! shortcut is on, and talks to it in JSON lines (`protocol.rs`). It is post-only: it never reads
//! the keyboard, which stays the listen-only `biorouter-fnkey`'s job, so the two macOS grants stay
//! with two programs that each do one thing.
//!
//! To insert, it puts the text on the pasteboard for a moment (marked transient and concealed, and
//! kept off Universal Clipboard), posts one Command-V to the app's own process, then puts back what
//! the pasteboard held, unless something else was copied in the meantime, and answers how the
//! clipboard was left (`pasteboard.rs`). A password the pasteboard held is not put back: the
//! pasteboard is cleared instead, so a password manager's own clearing still works. A long text
//! stays there longer before the restore (`restore_delay`), so a slow app pastes it and not the
//! old contents. One insert finishes, its restore included, before the next begins.
//!
//! Posting events needs macOS **Accessibility**. The helper reads it with
//! `CGPreflightPostEventAccess`, which never prompts; the prompt comes only from
//! `request-access`, which the app sends from a person's click in Settings. Started by the app,
//! the program is attributed to it, so the permission is the one listed as Biorouter.
//!
//! It exits when its standard input closes, after finishing (and restoring) an insert already
//! under way, so it never outlives the app.

// Built everywhere so its tests run on every CI leg; only macOS calls it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod protocol;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod pasteboard;

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    // Only macOS has a way for this helper to type into another app; the app never starts it
    // elsewhere, and copies the text instead.
    eprintln!("biorouter-textinsert is macOS-only");
    std::process::exit(2);
}

#[cfg(test)]
mod source_guards {
    //! What the helper may never do, read from its own source.

    /// Source lines that are code, not comments.
    fn code(source: &str) -> String {
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    const MAC: &str = include_str!("mac.rs");
    const PASTEBOARD: &str = include_str!("pasteboard.rs");
    const PROTOCOL: &str = include_str!("protocol.rs");
    const MAIN: &str = include_str!("main.rs");

    #[test]
    fn only_request_access_asks_macos_for_accessibility() {
        // Built from two halves so this file's own text does not count.
        let ask = concat!("CGRequestPost", "EventAccess(");
        assert_eq!(code(MAC).matches(ask).count(), 1);
        for other in [PASTEBOARD, PROTOCOL, MAIN] {
            assert_eq!(code(other).matches(ask).count(), 0);
        }
        // And that one call sits in the request-access arm.
        let mac = code(MAC);
        let arm = mac
            .find("Op::RequestAccess =>")
            .expect("the request-access arm");
        let call = mac.find(ask).expect("the call");
        assert!(
            call > arm && call - arm < 400,
            "the call belongs to its arm"
        );
    }

    #[test]
    fn the_general_pasteboard_is_named_once_and_never_in_a_test() {
        let general = concat!("general", "Pasteboard");
        assert_eq!(code(MAC).matches(general).count(), 1);
        for other in [PASTEBOARD, PROTOCOL, MAIN] {
            assert_eq!(code(other).matches(general).count(), 0);
        }
        // The one place is the pasteboard the helper writes when no test pasteboard is named.
        let tests = MAC.find("#[cfg(test)]").unwrap_or(MAC.len());
        assert!(MAC.find(general).is_some_and(|at| at < tests));
    }

    #[test]
    fn the_helper_never_reads_the_keyboard() {
        for source in [MAC, PASTEBOARD, PROTOCOL, MAIN] {
            let code = code(source);
            for listener in [
                concat!("CGEventTap", "Create"),
                concat!("CGRequestListen", "EventAccess"),
                concat!("CGPreflightListen", "EventAccess"),
                concat!("addGlobalMonitor", "ForEvents"),
            ] {
                assert!(!code.contains(listener), "{listener}");
            }
        }
    }

    #[test]
    fn nothing_writes_text_to_a_log() {
        for source in [MAC, PASTEBOARD, PROTOCOL] {
            for line in code(source).lines() {
                if line.contains("eprintln!") || line.contains("println!") {
                    // Every message is a fixed sentence: no argument is formatted into it.
                    assert!(
                        !line.contains("{"),
                        "a log line formats a value: {}",
                        line.trim()
                    );
                }
            }
        }
    }
}
