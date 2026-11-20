//! What the helper tells the desktop app, and how it reads the keyboard into it.
//!
//! The helper sees every key press on the Mac while it listens, so this module is
//! where its privacy is decided: only three facts ever leave the process. The fn
//! key went down, it came up, and some other key or modifier was pressed while it
//! was down or within [`TAP_FOLLOW_MS`] of its release (only the first such key
//! after each release, so a tap followed by typing is not read as half of a
//! double-tap). No key code or character is ever written, and no other key's
//! timing beyond that one.
//! The app reads one JSON object per line (`ui/desktop/src/utils/fnKeyHelper.ts`,
//! `parseFnHelperLine`), and writes one command per line back.

/// `kCGEventFlagMaskSecondaryFn`: set while fn (the globe key) is held.
pub const SECONDARY_FN: u64 = 0x0080_0000;

/// Shift, Control, Option and Command (`kCGEventFlagMask*`). Caps Lock and the
/// numeric pad flag are left out: neither is a modifier a person holds with fn.
pub const OTHER_MODIFIERS: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x0010_0000;

/// `kVK_Function`, and the code some recent Apple keyboards report for the
/// globe key.
pub const FN_KEY_CODES: [i64; 2] = [63, 179];

pub const PROTOCOL_VERSION: u32 = 1;

/// How long after fn is released the next key still counts as the gesture's
/// business: the app's double-tap gap (`FN_DOUBLE_TAP_GAP_MS` in
/// `ui/desktop/src/utils/fnGesture.ts`), so fn, a letter, fn is two taps.
pub const TAP_FOLLOW_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    FnDown,
    FnUp,
    /// A key or another modifier while fn is down: fn is being used as a
    /// modifier (fn with an arrow, Delete, an F key or a letter), so the app
    /// drops the gesture. Also the first one within [`TAP_FOLLOW_MS`] after fn
    /// is released, which ends the app's wait for a second tap.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Read Input Monitoring again and answer with a `permission` line.
    Check,
    /// Ask macOS for Input Monitoring (the system prompt, the first time only),
    /// then answer as `Check` does. Sent only from a person's click in Settings.
    Request,
    Quit,
}

pub fn parse_command(line: &str) -> Option<Command> {
    match line.trim() {
        "check" => Some(Command::Check),
        "request" => Some(Command::Request),
        "quit" => Some(Command::Quit),
        _ => None,
    }
}

/// Follows fn through the event stream. It holds only whether fn is down,
/// which other modifiers are, and when fn was last released until the next key
/// comes, never a key code or character.
#[derive(Debug, Default)]
pub struct FnTracker {
    fn_down: bool,
    modifiers: u64,
    /// When fn was last released, until the first key or modifier after it.
    released_at: Option<u64>,
}

impl FnTracker {
    pub const fn new() -> Self {
        Self {
            fn_down: false,
            modifiers: 0,
            released_at: None,
        }
    }

    /// A `kCGEventFlagsChanged` event at `at_ms`: a modifier went down or up.
    pub fn flags_changed(&mut self, key_code: i64, flags: u64, at_ms: u64) -> Vec<Signal> {
        let fn_now = flags & SECONDARY_FN != 0;
        let modifiers = flags & OTHER_MODIFIERS;
        let added = modifiers & !self.modifiers != 0;
        self.modifiers = modifiers;
        if FN_KEY_CODES.contains(&key_code) {
            if fn_now == self.fn_down {
                return Vec::new();
            }
            self.fn_down = fn_now;
            if !fn_now {
                self.released_at = Some(at_ms);
                return vec![Signal::FnUp];
            }
            self.released_at = None;
            // fn pressed while another modifier is already held is a shortcut
            // of its own, never a dictation gesture.
            return if modifiers != 0 {
                vec![Signal::FnDown, Signal::Other]
            } else {
                vec![Signal::FnDown]
            };
        }
        if self.fn_down && !fn_now {
            // The fn release was not seen (secure input took it, for one), but
            // the flags say fn is up now.
            self.fn_down = false;
            self.released_at = Some(at_ms);
            return vec![Signal::FnUp];
        }
        if added {
            self.other(at_ms).into_iter().collect()
        } else {
            Vec::new()
        }
    }

    /// A `kCGEventKeyDown` event at `at_ms`. Only whether fn was down, or just
    /// released, is used.
    pub fn key_down(&mut self, at_ms: u64) -> Option<Signal> {
        self.other(at_ms)
    }

    /// Another key or modifier: reported while fn is down, and once within
    /// [`TAP_FOLLOW_MS`] of its release. Any key after a release ends that
    /// window, so at most one key's timing leaves per release.
    fn other(&mut self, at_ms: u64) -> Option<Signal> {
        if self.fn_down {
            return Some(Signal::Other);
        }
        let released = self.released_at.take()?;
        (at_ms.saturating_sub(released) <= TAP_FOLLOW_MS).then_some(Signal::Other)
    }
}

/// A signal and when the event happened, in milliseconds on the event's own clock (Quartz's
/// timestamp, from system start). The app times gestures by it, so a burst of lines that reaches
/// a busy app late keeps its spacing: a hold read late is still a hold.
pub fn signal_line(signal: Signal, at_ms: u64) -> String {
    match signal {
        Signal::FnDown => format!(r#"{{"type":"fn","down":true,"t":{at_ms}}}"#),
        Signal::FnUp => format!(r#"{{"type":"fn","down":false,"t":{at_ms}}}"#),
        Signal::Other => format!(r#"{{"type":"other","t":{at_ms}}}"#),
    }
}

fn permission_word(permission: Permission) -> &'static str {
    match permission {
        Permission::Granted => "granted",
        Permission::Denied => "denied",
    }
}

/// `AppleFnUsageType` from `com.apple.HIToolbox`: what macOS itself does when fn
/// is pressed (0 nothing, 1 change the input source, 2 the emoji picker, 3 the
/// system's own dictation). Anything else is reported as unknown.
fn fn_usage_json(fn_usage: Option<i64>) -> String {
    match fn_usage {
        Some(value @ 0..=3) => value.to_string(),
        _ => "null".to_string(),
    }
}

/// The first line: the protocol version, Input Monitoring, whether the event
/// listener is running, what macOS does with fn, and the time now on the events'
/// clock, which lets the app line that clock up with its own before any key arrives.
pub fn ready_line(
    permission: Permission,
    listening: bool,
    fn_usage: Option<i64>,
    now_ms: u64,
) -> String {
    format!(
        r#"{{"type":"ready","v":{},"permission":"{}","listening":{},"fnUsage":{},"t":{}}}"#,
        PROTOCOL_VERSION,
        permission_word(permission),
        listening,
        fn_usage_json(fn_usage),
        now_ms
    )
}

pub fn permission_line(permission: Permission) -> String {
    format!(
        r#"{{"type":"permission","permission":"{}"}}"#,
        permission_word(permission)
    )
}

/// macOS turned the listener off and it could not be turned on again.
pub const LISTENER_LOST_LINE: &str = r#"{"type":"listener-lost"}"#;

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u64 = 0x0002_0000;
    const COMMAND: u64 = 0x0010_0000;
    const SHIFT_KEY: i64 = 56;
    const COMMAND_KEY: i64 = 55;

    /// Far enough after any release that no key counts as following it.
    const LATER: u64 = 100_000;

    #[test]
    fn fn_down_and_up_are_reported_once_each() {
        let mut tracker = FnTracker::new();
        assert_eq!(
            tracker.flags_changed(63, SECONDARY_FN, 0),
            vec![Signal::FnDown]
        );
        assert_eq!(tracker.flags_changed(63, SECONDARY_FN, 10), vec![]);
        assert_eq!(tracker.flags_changed(63, 0, 20), vec![Signal::FnUp]);
        assert_eq!(tracker.flags_changed(63, 0, 30), vec![]);
    }

    #[test]
    fn the_globe_key_code_counts_as_fn() {
        let mut tracker = FnTracker::new();
        assert_eq!(
            tracker.flags_changed(179, SECONDARY_FN, 0),
            vec![Signal::FnDown]
        );
        assert_eq!(tracker.flags_changed(179, 0, 10), vec![Signal::FnUp]);
    }

    #[test]
    fn a_key_while_fn_is_down_is_other_and_says_nothing_else() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.key_down(0), None);
        tracker.flags_changed(63, SECONDARY_FN, 10);
        assert_eq!(tracker.key_down(20), Some(Signal::Other));
        tracker.flags_changed(63, 0, 30);
        assert_eq!(tracker.key_down(LATER), None);
    }

    /// fn, a letter, fn within the double-tap gap is two taps: the first key
    /// after a release is reported, once, and only within the gap.
    #[test]
    fn the_first_key_soon_after_a_release_is_other_and_no_later_one_is() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, 1_000);
        tracker.flags_changed(63, 0, 1_080);
        assert_eq!(tracker.key_down(1_080 + TAP_FOLLOW_MS), Some(Signal::Other));
        assert_eq!(tracker.key_down(1_080 + TAP_FOLLOW_MS + 1), None);

        tracker.flags_changed(63, SECONDARY_FN, 2_000);
        tracker.flags_changed(63, 0, 2_080);
        assert_eq!(tracker.key_down(2_080 + TAP_FOLLOW_MS + 1), None);
        assert_eq!(tracker.key_down(2_080 + TAP_FOLLOW_MS + 2), None);

        tracker.flags_changed(63, SECONDARY_FN, 3_000);
        tracker.flags_changed(63, 0, 3_080);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SHIFT, 3_100),
            vec![Signal::Other]
        );
        assert_eq!(tracker.key_down(3_120), None);

        // While fn is down again, every key is reported, as before any release.
        tracker.flags_changed(63, 0, 3_200);
        tracker.flags_changed(63, SECONDARY_FN, 4_000);
        tracker.flags_changed(63, 0, 4_050);
        tracker.flags_changed(63, SECONDARY_FN, 4_100);
        assert_eq!(tracker.key_down(4_150), Some(Signal::Other));
    }

    #[test]
    fn a_modifier_pressed_while_fn_is_down_is_other_and_its_release_is_not() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, 0);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SECONDARY_FN | SHIFT, 10),
            vec![Signal::Other]
        );
        assert_eq!(tracker.flags_changed(SHIFT_KEY, SECONDARY_FN, 20), vec![]);
    }

    #[test]
    fn fn_pressed_while_command_is_held_is_down_then_other() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.flags_changed(COMMAND_KEY, COMMAND, 0), vec![]);
        assert_eq!(
            tracker.flags_changed(63, COMMAND | SECONDARY_FN, 10),
            vec![Signal::FnDown, Signal::Other]
        );
    }

    #[test]
    fn a_missed_fn_release_is_recovered_from_the_next_flags() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, 0);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SHIFT, 10),
            vec![Signal::FnUp]
        );
        assert_eq!(tracker.key_down(LATER), None);
    }

    #[test]
    fn modifiers_without_fn_report_nothing() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.flags_changed(SHIFT_KEY, SHIFT, 0), vec![]);
        assert_eq!(tracker.flags_changed(SHIFT_KEY, 0, 10), vec![]);
        assert_eq!(tracker.key_down(20), None);
    }

    #[test]
    fn the_lines_are_the_ones_the_app_parses() {
        assert_eq!(
            signal_line(Signal::FnDown, 1200),
            r#"{"type":"fn","down":true,"t":1200}"#
        );
        assert_eq!(
            signal_line(Signal::FnUp, 1500),
            r#"{"type":"fn","down":false,"t":1500}"#
        );
        assert_eq!(signal_line(Signal::Other, 7), r#"{"type":"other","t":7}"#);
        assert_eq!(
            ready_line(Permission::Granted, true, Some(2), 900),
            r#"{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":2,"t":900}"#
        );
        assert_eq!(
            ready_line(Permission::Denied, false, None, 0),
            r#"{"type":"ready","v":1,"permission":"denied","listening":false,"fnUsage":null,"t":0}"#
        );
        assert_eq!(
            ready_line(Permission::Granted, true, Some(9), 1),
            r#"{"type":"ready","v":1,"permission":"granted","listening":true,"fnUsage":null,"t":1}"#
        );
        assert_eq!(
            permission_line(Permission::Granted),
            r#"{"type":"permission","permission":"granted"}"#
        );
        assert_eq!(LISTENER_LOST_LINE, r#"{"type":"listener-lost"}"#);
    }

    #[test]
    fn commands_are_exact_words() {
        assert_eq!(parse_command("check\n"), Some(Command::Check));
        assert_eq!(parse_command(" request "), Some(Command::Request));
        assert_eq!(parse_command("quit"), Some(Command::Quit));
        assert_eq!(parse_command("CHECK"), None);
        assert_eq!(parse_command("check now"), None);
        assert_eq!(parse_command(""), None);
    }
}
