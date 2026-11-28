//! The macOS fn key listener behind Biorouter's dictation gestures (hold fn to
//! talk, double-tap fn for hands-free, and the same with Shift, Control or Option
//! held when another app also uses fn).
//!
//! Electron cannot register fn as a global shortcut, and Chromium does not
//! deliver it to a page reliably, so the desktop app starts this program and
//! reads what it reports on standard output (`protocol.rs`). It is a listen-only
//! Quartz event tap: it can see key presses but never change, block or make one.
//! A timer on the tap's own run loop says when fn has stayed down long enough to
//! be a hold (`held`), so the app learns of a hold on the events' clock and in
//! order with the release, however busy it is. The helper decides no policy: it
//! says which modifier was held with fn, and the app decides which presses
//! dictate.
//!
//! On a thread of its own it also reads which other apps hold an event tap on
//! modifier keys (`CGGetEventTapList`, which needs no permission and creates
//! nothing) and reports their bundle identifiers, so the app can tell when an
//! app it knows uses fn, Wispr Flow for one, is running.
//!
//! The tap needs macOS **Input Monitoring**. The program reads that permission
//! with `CGPreflightListenEventAccess`, which never shows a prompt, and creates
//! the tap only when it is granted, because creating one without it is what
//! makes macOS ask. The prompt comes only from `request`, which the app sends
//! from a person's click in Settings. Started by the app, the program is
//! attributed to it, so the permission is the one listed as Biorouter.
//!
//! It exits when its standard input closes, so it never outlives the app.

// Built everywhere so their tests run on every CI leg; only macOS calls them.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod outbox;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod protocol;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    // Only macOS lets an app see the fn key; the app never starts this elsewhere.
    eprintln!("biorouter-fnkey is macOS-only");
    std::process::exit(64);
}

#[cfg(test)]
mod source_guards {
    //! What the helper may never do, read from its own source.

    const MAC: &str = include_str!("mac.rs");
    const OUTBOX: &str = include_str!("outbox.rs");
    const PROTOCOL: &str = include_str!("protocol.rs");
    const MAIN: &str = include_str!("main.rs");

    /// Source lines that are code, not comments, before the file's tests: a test may read
    /// the system the way the watch does, and is not what the helper runs.
    fn code(source: &str) -> String {
        let shipped = source
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or(source);
        shipped
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The top-level item of `source` that defines `fn <name>(`: from its line to the next
    /// line that starts at column 0 with something other than a closing brace or the closing
    /// parenthesis of a signature written over several lines.
    fn item<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("fn {name}(");
        let (before, after) = source
            .split_once(&signature)
            .unwrap_or_else(|| panic!("no fn {name}"));
        // The signature's own line, then every line up to the next item.
        let own_line = before.rsplit('\n').next().unwrap_or_default();
        let start = before.len() - own_line.len();
        let mut length = own_line.len() + signature.len();
        for line in after.split_inclusive('\n').skip(1) {
            if line
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace() && c != '}' && c != ')')
            {
                break;
            }
            length += line.len();
        }
        let first = after.split_inclusive('\n').next().unwrap_or_default();
        length += first.len();
        source
            .get(start..start + length)
            .expect("an item on character boundaries")
    }

    /// Calls of `name`: every `name(` that is not its own declaration.
    fn calls(source: &str, name: &str) -> usize {
        let needle = format!("{name}(");
        source.matches(&needle).count() - source.matches(&format!("fn {needle}")).count()
    }

    #[test]
    fn one_tap_is_created_and_only_a_click_asks_macos_for_input_monitoring() {
        // Built from two halves so this file's own text does not count.
        let create = concat!("CGEventTap", "Create");
        let ask = concat!("CGRequestListen", "EventAccess");
        let mac = code(MAC);
        assert_eq!(calls(&mac, create), 1, "one tap, created in run");
        assert!(calls(item(&mac, "run"), create) == 1);
        assert_eq!(calls(&mac, ask), 1, "one request, in the request arm");
        let arm = mac
            .find("Some(Command::Request) =>")
            .expect("the request arm");
        let call = mac
            .get(arm..)
            .and_then(|from_arm| from_arm.find(&format!("unsafe {{ {ask}() }}")))
            .expect("the call, after its arm");
        assert!(call < 300, "the call belongs to its arm");
        for other in [OUTBOX, PROTOCOL, MAIN] {
            let other = code(other);
            assert_eq!(other.matches(create).count(), 0);
            assert_eq!(other.matches(ask).count(), 0);
        }
    }

    /// The listener watch reads the list of taps on its own thread: the functions that read
    /// it are called only from the watch, the watch only by a thread spawn, and nothing that
    /// runs on the tap's run loop names any of them.
    #[test]
    fn the_listener_watch_never_runs_on_the_taps_run_loop() {
        let mac = code(MAC);
        let list = concat!("CGGetEvent", "TapList");
        assert_eq!(calls(&mac, list), 2, "the count, then the list");
        assert_eq!(calls(item(&mac, "read_tap_list"), list), 2);
        for (reader, only_in) in [
            ("read_tap_list", "read_listeners"),
            ("process_path", "read_listeners"),
            ("read_bundle_identifier", "of"),
            ("read_listeners", "watch_listeners"),
        ] {
            assert_eq!(calls(&mac, reader), 1, "{reader} is called once");
            assert_eq!(
                calls(item(&mac, only_in), reader),
                1,
                "{reader} from {only_in}"
            );
        }
        assert_eq!(calls(&mac, "proc_pidpath"), 1);
        assert_eq!(calls(&mac, "CFBundleCreate"), 1);
        assert_eq!(calls(item(&mac, "process_path"), "proc_pidpath"), 1);
        assert_eq!(
            calls(item(&mac, "read_bundle_identifier"), "CFBundleCreate"),
            1
        );
        // The watch is started by a spawn, once, and never called.
        assert_eq!(
            mac.matches("watch_listeners").count(),
            2,
            "its definition and its spawn"
        );
        assert_eq!(
            item(&mac, "announce_ready")
                .matches("std::thread::spawn(watch_listeners)")
                .count(),
            1
        );
        // What runs on the tap's thread names none of the readers, the watch, or its wait.
        for on_the_run_loop in [
            "on_event",
            "on_tap_disabled",
            "on_hold_timer",
            "give_held",
            "track",
            "follow_hold",
            "arm_hold_timer",
            "cancel_hold_timer",
            "emit",
            "emit_signal",
        ] {
            let body = item(&mac, on_the_run_loop);
            for reader in [
                list,
                "read_tap_list",
                "read_listeners",
                "process_path",
                "read_bundle_identifier",
                "proc_pidpath",
                "CFBundleCreate",
                "wait_for_listener_ask",
                "watch_listeners",
            ] {
                assert!(!body.contains(reader), "{on_the_run_loop} names {reader}");
            }
        }
        // The command only asks: the reading stays on the watch's thread.
        let serve = item(&mac, "serve_commands");
        assert!(serve.contains("Some(Command::Listeners) => ask_for_listeners()"));
        for reader in ["read_tap_list", "read_listeners", list] {
            assert!(!serve.contains(reader), "serve_commands names {reader}");
        }
    }

    /// A line carries no key code, repeat flag, event source or character: no line maker
    /// formats any of what the tap reads.
    #[test]
    fn no_line_carries_what_the_tap_reads() {
        let protocol = code(PROTOCOL);
        for maker in [
            "signal_line",
            "ready_line",
            "permission_line",
            "listeners_line",
        ] {
            let body = item(&protocol, maker);
            for fact in ["key_code", "autorepeat", "source", "KeyFacts", "flags"] {
                assert!(!body.contains(fact), "{maker} names {fact}");
            }
        }
    }
}
