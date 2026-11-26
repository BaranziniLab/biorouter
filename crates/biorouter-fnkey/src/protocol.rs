//! What the helper tells the desktop app, and how it reads the keyboard into it.
//!
//! The helper sees every key press on the Mac while it listens, so this module is
//! where its privacy is decided: only four facts ever leave the process. The fn
//! key went down, it came up, it is still down [`FN_HOLD_MS`] after it went down
//! with nothing else pressed (`held`), and some other key or modifier was pressed
//! while it was down or within [`TAP_FOLLOW_MS`] of its release. While fn is held,
//! every other key press is reported as `other` with its time; after a release only
//! the first real key counts (one from the keyboard itself, not fn or the globe
//! key, not a repeat), so a tap followed by typing is not read as half of a
//! double-tap, and a key that macOS or another app makes up never ends one. To tell
//! a real key from the rest, the tap reads a key's code, its repeat flag and its
//! source ([`KeyFacts`]); the tracker judges them on the spot and keeps none. No
//! key code, repeat flag, source or character is ever written, so a key's identity
//! never leaves the process, and outside an fn hold no key's timing leaves beyond
//! that first one. `held` says only how long fn itself was down.
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

/// Version 2 adds the `held` line and the narrower `other` after a release. The app
/// refuses a ready line of any other version, so a helper and an app always agree.
pub const PROTOCOL_VERSION: u32 = 2;

/// `kCGEventKeyDown`.
pub const EVENT_KEY_DOWN: u32 = 10;
/// `kCGEventFlagsChanged`: a modifier, fn among them, went down or up.
pub const EVENT_FLAGS_CHANGED: u32 = 12;
/// `kCGEventTapDisabledByTimeout`: macOS turned the tap off because the callback was slow.
pub const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
/// `kCGEventTapDisabledByUserInput`: macOS turned the tap off while secure input was on.
pub const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

/// The events the tap asks for: key down and flags changed, nothing else (no key up,
/// no mouse). The two disabled kinds arrive whatever the mask says.
pub const EVENT_MASK: u64 = (1u64 << EVENT_KEY_DOWN) | (1u64 << EVENT_FLAGS_CHANGED);
const _: () = assert!(EVENT_MASK == 0x1400);

/// What the tap's callback does with an event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    /// macOS turned the tap off. The event pointer is NULL for these, so nothing may read it.
    Disabled,
    FlagsChanged,
    KeyDown,
    Ignore,
}

pub fn classify_event(kind: u32) -> EventClass {
    match kind {
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT => EventClass::Disabled,
        EVENT_FLAGS_CHANGED => EventClass::FlagsChanged,
        EVENT_KEY_DOWN => EventClass::KeyDown,
        _ => EventClass::Ignore,
    }
}

/// How long after fn is released the next key still counts as the gesture's
/// business: the app's double-tap gap (`FN_DOUBLE_TAP_GAP_MS` in
/// `ui/desktop/src/utils/fnGesture.ts`), so fn, a letter, fn is two taps.
pub const TAP_FOLLOW_MS: u64 = 400;

/// How long fn stays down, with nothing else pressed, before the helper says `held`:
/// the app's hold threshold (`FN_HOLD_MS` in `ui/desktop/src/utils/fnGesture.ts`). The
/// helper times it on the events' own clock, so a hold is decided in order with the
/// release however late the app reads either line.
pub const FN_HOLD_MS: u64 = 300;

/// The oldest an event's own time may be, against the clock now, and still be used.
/// Anything older, in the future, or 0 (what a made-up event carries) is timed now.
pub const MAX_EVENT_AGE_MS: u64 = 10_000;

/// `kCGEventSourceStateHIDSystemState`: the source state of events that come from the
/// keyboard itself, read from `kCGEventSourceStateID`.
pub const HID_SYSTEM_STATE: i64 = 1;

/// The time of an event in milliseconds on the events' clock. `ts_ns` is Quartz's
/// timestamp (nanoseconds since start-up, not counting sleep), `now_ms` the same clock
/// now. A timestamp of 0, one after now, or one more than [`MAX_EVENT_AGE_MS`] before
/// now is not believed, and the event is timed now instead: an event a program made
/// without stamping it carries 0, which would otherwise make every press look old
/// enough to be a hold the moment it went down.
pub fn event_ms(ts_ns: u64, now_ms: u64) -> u64 {
    let at_ms = ts_ns / 1_000_000;
    if ts_ns != 0 && at_ms <= now_ms && now_ms - at_ms <= MAX_EVENT_AGE_MS {
        at_ms
    } else {
        now_ms
    }
}

/// Where a key event came from, as far as the gestures care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The keyboard itself ([`HID_SYSTEM_STATE`]).
    Hardware,
    /// Made by a program: macOS, another app, a test.
    Synthesized,
}

impl Source {
    /// From an event's `kCGEventSourceStateID`.
    pub fn of_state(state_id: i64) -> Self {
        if state_id == HID_SYSTEM_STATE {
            Self::Hardware
        } else {
            Self::Synthesized
        }
    }
}

/// What the tap reads from a key down to decide whether it is another key. The
/// tracker judges these on the spot and keeps none of them, and nothing in them is
/// ever written to the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyFacts {
    /// `kCGKeyboardEventKeycode`.
    pub key_code: i64,
    /// `kCGKeyboardEventAutorepeat`: the key is being held and this is a repeat.
    pub autorepeat: bool,
    pub source: Source,
}

impl KeyFacts {
    /// A key down of fn or the globe key itself, which some keyboards and macOS
    /// deliver around the flags change of a press.
    fn is_fn_key(self) -> bool {
        FN_KEY_CODES.contains(&self.key_code)
    }

    /// Whether, after a release, this key ends the wait for a second tap: a fresh
    /// press from the keyboard. A repeat belongs to a key pressed before, and a key a
    /// program makes up is no sign that a person moved on.
    fn counts_after_release(self) -> bool {
        !self.autorepeat && self.source == Source::Hardware
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    FnDown,
    FnUp,
    /// fn has stayed down [`FN_HOLD_MS`] with nothing else pressed. Written with the
    /// time the threshold was reached (the press's down time plus [`FN_HOLD_MS`]).
    Held,
    /// A key or another modifier while fn is down: fn is being used as a
    /// modifier (fn with an arrow, Delete, an F key or a letter), so the app
    /// drops the gesture. Also the first real key within [`TAP_FOLLOW_MS`] after
    /// fn is released, which ends the app's wait for a second tap.
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

/// Follows fn through the event stream. It holds only whether fn is down, which
/// other modifiers are, when fn was last released until the next key comes, and when
/// the press still waiting for `held` went down, never a key code or character.
#[derive(Debug, Default)]
pub struct FnTracker {
    fn_down: bool,
    modifiers: u64,
    /// When fn was last released, until the first real key or modifier after it.
    released_at: Option<u64>,
    /// When the press now down went down, while it may still become a hold: nothing
    /// else was pressed with it and `held` has not been given for it yet.
    hold_from: Option<u64>,
}

impl FnTracker {
    pub const fn new() -> Self {
        Self {
            fn_down: false,
            modifiers: 0,
            released_at: None,
            hold_from: None,
        }
    }

    /// A `kCGEventFlagsChanged` event at `at_ms`: a modifier went down or up. `source`
    /// matters only for a modifier added after a release, which counts only from the
    /// keyboard itself.
    pub fn flags_changed(
        &mut self,
        key_code: i64,
        flags: u64,
        source: Source,
        at_ms: u64,
    ) -> Vec<Signal> {
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
                self.release(at_ms);
                return vec![Signal::FnUp];
            }
            self.released_at = None;
            // fn pressed while another modifier is already held is a shortcut
            // of its own, never a dictation gesture.
            return if modifiers != 0 {
                self.hold_from = None;
                vec![Signal::FnDown, Signal::Other]
            } else {
                self.hold_from = Some(at_ms);
                vec![Signal::FnDown]
            };
        }
        if self.fn_down && !fn_now {
            // The fn release was not seen (secure input took it, for one), but
            // the flags say fn is up now.
            self.fn_down = false;
            self.release(at_ms);
            return vec![Signal::FnUp];
        }
        if added {
            self.other(at_ms, source == Source::Hardware)
                .into_iter()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// A `kCGEventKeyDown` event at `at_ms`. While fn is down, any key but fn or the
    /// globe key itself is another key, from any source and repeats included; after a
    /// release only a real one is ([`KeyFacts`]). The facts are judged here and kept
    /// nowhere.
    pub fn key_down(&mut self, at_ms: u64, facts: KeyFacts) -> Option<Signal> {
        if facts.is_fn_key() {
            return None;
        }
        self.other(at_ms, facts.counts_after_release())
    }

    /// When the press now down will have been held for [`FN_HOLD_MS`], while it may
    /// still become a hold. The tap arms its timer for this time.
    pub fn hold_deadline(&self) -> Option<u64> {
        self.hold_from.map(|down| down.saturating_add(FN_HOLD_MS))
    }

    /// Asked when the timer fires, at `now_ms` on the events' clock: the time to write
    /// in a `held` line, once per press, when the press is still down and unspoiled and
    /// its threshold has passed. `None` before the threshold, for a press already
    /// released or spoiled, and after it was given once.
    pub fn held_due(&mut self, now_ms: u64) -> Option<u64> {
        let deadline = self.hold_deadline()?;
        if now_ms < deadline {
            return None;
        }
        self.hold_from = None;
        Some(deadline)
    }

    /// macOS turned the tap off and it was turned on again, so the tracker was blind for
    /// a while and may have missed fn's release. Forget everything it held, and report
    /// fn up if it believed fn was down: a hold the app would otherwise keep recording
    /// until some later modifier event ends, at worst, a hold early. A release the
    /// tracker then sees is no change and reports nothing. A press waiting for `held`
    /// never gets it.
    pub fn reset_after_blind_gap(&mut self) -> Option<Signal> {
        let was_down = self.fn_down;
        self.fn_down = false;
        self.modifiers = 0;
        self.released_at = None;
        self.hold_from = None;
        was_down.then_some(Signal::FnUp)
    }

    fn release(&mut self, at_ms: u64) {
        self.released_at = Some(at_ms);
        self.hold_from = None;
    }

    /// Another key or modifier: reported while fn is down, which spoils the press, and
    /// once within [`TAP_FOLLOW_MS`] of its release when `counts_after_release`. Such a
    /// key after a release ends that window, so at most one key's timing leaves per
    /// release; a key that does not count leaves the window open for one that does.
    fn other(&mut self, at_ms: u64, counts_after_release: bool) -> Option<Signal> {
        if self.fn_down {
            self.hold_from = None;
            return Some(Signal::Other);
        }
        if !counts_after_release {
            return None;
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
        Signal::Held => format!(r#"{{"type":"held","t":{at_ms}}}"#),
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
    const HW: Source = Source::Hardware;
    const MADE_UP: Source = Source::Synthesized;
    /// `kVK_ANSI_A`, and `kVK_LeftArrow`.
    const A_KEY: i64 = 0;
    const LEFT_ARROW: i64 = 123;

    /// Far enough after any release that no key counts as following it.
    const LATER: u64 = 100_000;

    fn key(key_code: i64, source: Source, autorepeat: bool) -> KeyFacts {
        KeyFacts {
            key_code,
            autorepeat,
            source,
        }
    }

    /// A fresh press of a letter on the keyboard.
    fn letter() -> KeyFacts {
        key(A_KEY, HW, false)
    }

    /// A tracker that has just seen a tap of fn from `down` to `up`.
    fn tapped(down: u64, up: u64) -> FnTracker {
        let mut tracker = FnTracker::new();
        assert_eq!(
            tracker.flags_changed(63, SECONDARY_FN, HW, down),
            vec![Signal::FnDown]
        );
        assert_eq!(tracker.flags_changed(63, 0, HW, up), vec![Signal::FnUp]);
        tracker
    }

    #[test]
    fn fn_down_and_up_are_reported_once_each() {
        let mut tracker = FnTracker::new();
        assert_eq!(
            tracker.flags_changed(63, SECONDARY_FN, HW, 0),
            vec![Signal::FnDown]
        );
        assert_eq!(tracker.flags_changed(63, SECONDARY_FN, HW, 10), vec![]);
        assert_eq!(tracker.flags_changed(63, 0, HW, 20), vec![Signal::FnUp]);
        assert_eq!(tracker.flags_changed(63, 0, HW, 30), vec![]);
    }

    /// fn itself counts from any source, so a stamped test press still reaches the app.
    #[test]
    fn the_globe_key_code_counts_as_fn_from_any_source() {
        let mut tracker = FnTracker::new();
        assert_eq!(
            tracker.flags_changed(179, SECONDARY_FN, MADE_UP, 0),
            vec![Signal::FnDown]
        );
        assert_eq!(
            tracker.flags_changed(179, 0, MADE_UP, 10),
            vec![Signal::FnUp]
        );
    }

    #[test]
    fn a_key_while_fn_is_down_is_other_and_says_nothing_else() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.key_down(0, letter()), None);
        tracker.flags_changed(63, SECONDARY_FN, HW, 10);
        assert_eq!(tracker.key_down(20, letter()), Some(Signal::Other));
        tracker.flags_changed(63, 0, HW, 30);
        assert_eq!(tracker.key_down(LATER, letter()), None);
    }

    /// fn, a letter, fn within the double-tap gap is two taps: the first key
    /// after a release is reported, once, and only within the gap.
    #[test]
    fn the_first_key_soon_after_a_release_is_other_and_no_later_one_is() {
        let mut tracker = tapped(1_000, 1_080);
        assert_eq!(
            tracker.key_down(1_080 + TAP_FOLLOW_MS, letter()),
            Some(Signal::Other)
        );
        assert_eq!(tracker.key_down(1_080 + TAP_FOLLOW_MS + 1, letter()), None);

        tracker.flags_changed(63, SECONDARY_FN, HW, 2_000);
        tracker.flags_changed(63, 0, HW, 2_080);
        assert_eq!(tracker.key_down(2_080 + TAP_FOLLOW_MS + 1, letter()), None);
        assert_eq!(tracker.key_down(2_080 + TAP_FOLLOW_MS + 2, letter()), None);

        tracker.flags_changed(63, SECONDARY_FN, HW, 3_000);
        tracker.flags_changed(63, 0, HW, 3_080);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SHIFT, HW, 3_100),
            vec![Signal::Other]
        );
        assert_eq!(tracker.key_down(3_120, letter()), None);

        // While fn is down again, every key is reported, as before any release.
        tracker.flags_changed(63, 0, HW, 3_200);
        tracker.flags_changed(63, SECONDARY_FN, HW, 4_000);
        tracker.flags_changed(63, 0, HW, 4_050);
        tracker.flags_changed(63, SECONDARY_FN, HW, 4_100);
        assert_eq!(tracker.key_down(4_150, letter()), Some(Signal::Other));
    }

    /// A key down of fn or the globe key itself, right after a release or just before the
    /// next press, is part of the gesture, never another key, and leaves the wait for a
    /// real key open. While fn is down it does not spoil the press either.
    #[test]
    fn a_globe_key_down_around_a_tap_is_not_another_key() {
        for code in FN_KEY_CODES {
            let mut tracker = tapped(1_000, 1_090);
            assert_eq!(tracker.key_down(1_100, key(code, HW, false)), None);
            // Just before the second press.
            assert_eq!(tracker.key_down(1_210, key(code, HW, false)), None);
            assert_eq!(
                tracker.flags_changed(63, SECONDARY_FN, HW, 1_220),
                vec![Signal::FnDown]
            );
            assert_eq!(tracker.key_down(1_230, key(code, HW, false)), None);
            assert_eq!(tracker.hold_deadline(), Some(1_220 + FN_HOLD_MS));
            assert_eq!(tracker.flags_changed(63, 0, HW, 1_310), vec![Signal::FnUp]);

            // The window it left open still ends at the first real key.
            let mut tracker = tapped(5_000, 5_090);
            assert_eq!(tracker.key_down(5_100, key(code, HW, false)), None);
            assert_eq!(tracker.key_down(5_150, letter()), Some(Signal::Other));
        }
    }

    /// A key that macOS or another app makes up after a release never ends a double-tap,
    /// and a modifier it adds does not either; the same key from the keyboard still does,
    /// so fn, a letter, fn stays two taps.
    #[test]
    fn a_synthesized_key_down_after_a_release_is_not_another_key() {
        for state in [0, -1, 2] {
            assert_eq!(Source::of_state(state), Source::Synthesized, "{state}");
        }
        assert_eq!(Source::of_state(HID_SYSTEM_STATE), Source::Hardware);

        let mut tracker = tapped(1_000, 1_090);
        assert_eq!(tracker.key_down(1_100, key(A_KEY, MADE_UP, false)), None);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, MADE_UP, 1_110),
            vec![]
        );
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, 0, MADE_UP, 1_120),
            vec![]
        );
        assert_eq!(
            tracker.key_down(1_130, key(A_KEY, HW, false)),
            Some(Signal::Other)
        );

        let mut tracker = tapped(2_000, 2_090);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SHIFT, HW, 2_100),
            vec![Signal::Other]
        );
    }

    #[test]
    fn an_autorepeat_key_down_is_not_another_key() {
        let mut tracker = tapped(1_000, 1_090);
        assert_eq!(tracker.key_down(1_100, key(A_KEY, HW, true)), None);
        assert_eq!(tracker.key_down(1_150, key(LEFT_ARROW, HW, true)), None);
        // The next fresh press still counts.
        assert_eq!(tracker.key_down(1_200, letter()), Some(Signal::Other));
    }

    /// While fn is down, a key is fn being used as a modifier whatever made it: the press
    /// is spoiled, so fn with an arrow never dictates.
    #[test]
    fn a_letter_while_fn_is_down_is_still_other() {
        for facts in [
            letter(),
            key(A_KEY, MADE_UP, false),
            key(A_KEY, HW, true),
            key(LEFT_ARROW, MADE_UP, true),
        ] {
            let mut tracker = FnTracker::new();
            tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
            assert_eq!(
                tracker.key_down(1_050, facts),
                Some(Signal::Other),
                "{facts:?}"
            );
            assert_eq!(tracker.held_due(1_000 + FN_HOLD_MS), None, "{facts:?}");
        }
        // A modifier added from any source spoils it too.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SECONDARY_FN | SHIFT, MADE_UP, 1_050),
            vec![Signal::Other]
        );
    }

    #[test]
    fn held_is_due_once_at_the_threshold_while_fn_stays_down() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.hold_deadline(), None);
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(tracker.hold_deadline(), Some(1_300));
        assert_eq!(tracker.held_due(1_299), None);
        assert_eq!(tracker.held_due(1_300), Some(1_300));
        assert_eq!(tracker.held_due(1_301), None);
        assert_eq!(tracker.hold_deadline(), None);
        // A timer that fires late still writes the threshold's time, once.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 2_000);
        assert_eq!(tracker.held_due(2_750), Some(2_300));
        assert_eq!(tracker.held_due(2_800), None);
        // The hold ends with an ordinary release, and the next press waits afresh.
        assert_eq!(tracker.flags_changed(63, 0, HW, 3_000), vec![Signal::FnUp]);
        tracker.flags_changed(63, SECONDARY_FN, HW, 4_000);
        assert_eq!(tracker.held_due(4_300), Some(4_300));
    }

    #[test]
    fn held_is_never_due_for_a_tap() {
        let mut tracker = tapped(1_000, 1_090);
        assert_eq!(tracker.hold_deadline(), None);
        assert_eq!(tracker.held_due(1_300), None);
        assert_eq!(tracker.held_due(LATER), None);
        // Nor for the second press of a double-tap once it is released, nor when a timer
        // armed for the first press fires during the second.
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_220);
        assert_eq!(tracker.held_due(1_300), None);
        tracker.flags_changed(63, 0, HW, 1_310);
        assert_eq!(tracker.held_due(1_520), None);
        // A release the flags of another key reveal ends the wait too.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 2_000);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SHIFT, HW, 2_100),
            vec![Signal::FnUp]
        );
        assert_eq!(tracker.held_due(2_300), None);
    }

    #[test]
    fn held_is_cancelled_by_another_key_or_a_blind_gap() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(tracker.key_down(1_100, letter()), Some(Signal::Other));
        assert_eq!(tracker.hold_deadline(), None);
        assert_eq!(tracker.held_due(1_300), None);

        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        tracker.flags_changed(COMMAND_KEY, SECONDARY_FN | COMMAND, HW, 1_100);
        assert_eq!(tracker.held_due(1_300), None);

        // fn pressed with Command already held is a shortcut from the start.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 900);
        tracker.flags_changed(63, COMMAND | SECONDARY_FN, HW, 1_000);
        assert_eq!(tracker.hold_deadline(), None);

        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(tracker.reset_after_blind_gap(), Some(Signal::FnUp));
        assert_eq!(tracker.hold_deadline(), None);
        assert_eq!(tracker.held_due(1_300), None);
    }

    #[test]
    fn implausible_event_times_fall_back_to_now() {
        let now = 50_000;
        let ns = |ms: u64| ms * 1_000_000;
        // What a made-up event carries.
        assert_eq!(event_ms(0, now), now);
        // Ahead of the clock.
        assert_eq!(event_ms(ns(now + 1), now), now);
        assert_eq!(event_ms(u64::MAX, now), now);
        // Older than the limit, or a nonsense value that rounds to almost 0.
        assert_eq!(event_ms(ns(now - MAX_EVENT_AGE_MS - 1), now), now);
        assert_eq!(event_ms(1, now), now);
        // A plausible time is kept, to the millisecond, at the limit too.
        assert_eq!(event_ms(ns(now - 120) + 999_999, now), now - 120);
        assert_eq!(event_ms(ns(now), now), now);
        assert_eq!(
            event_ms(ns(now - MAX_EVENT_AGE_MS), now),
            now - MAX_EVENT_AGE_MS
        );
        // Soon after start-up, a real time near 0 is still kept.
        assert_eq!(event_ms(ns(1_000), 5_000), 1_000);
    }

    #[test]
    fn a_modifier_pressed_while_fn_is_down_is_other_and_its_release_is_not() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 0);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SECONDARY_FN | SHIFT, HW, 10),
            vec![Signal::Other]
        );
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SECONDARY_FN, HW, 20),
            vec![]
        );
    }

    #[test]
    fn fn_pressed_while_command_is_held_is_down_then_other() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 0), vec![]);
        assert_eq!(
            tracker.flags_changed(63, COMMAND | SECONDARY_FN, HW, 10),
            vec![Signal::FnDown, Signal::Other]
        );
    }

    #[test]
    fn a_missed_fn_release_is_recovered_from_the_next_flags() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 0);
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SHIFT, HW, 10),
            vec![Signal::FnUp]
        );
        assert_eq!(tracker.key_down(LATER, letter()), None);
    }

    /// macOS turned the tap off while fn was down, so its release may have been missed:
    /// the reset reports fn up once, and the tracker starts clean.
    #[test]
    fn a_reset_after_a_blind_gap_ends_a_hold_once() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 100);
        assert_eq!(tracker.reset_after_blind_gap(), Some(Signal::FnUp));
        // Nothing is left of the hold or of a release window.
        assert_eq!(tracker.key_down(200, letter()), None);
        assert_eq!(tracker.reset_after_blind_gap(), None);
        // The real release, seen after the reset, is no change.
        assert_eq!(tracker.flags_changed(63, 0, HW, 300), vec![]);
        // And the next press is an ordinary fn down.
        assert_eq!(
            tracker.flags_changed(63, SECONDARY_FN, HW, 400),
            vec![Signal::FnDown]
        );
    }

    #[test]
    fn a_reset_with_fn_up_reports_nothing() {
        assert_eq!(FnTracker::new().reset_after_blind_gap(), None);
        let mut tracker = tapped(0, 10);
        assert_eq!(tracker.reset_after_blind_gap(), None);
        // The release window is gone too, so the next key is not reported.
        assert_eq!(tracker.key_down(20, letter()), None);
    }

    #[test]
    fn event_kinds_are_classified_as_the_callback_needs() {
        assert_eq!(classify_event(0xFFFF_FFFE), EventClass::Disabled);
        assert_eq!(classify_event(0xFFFF_FFFF), EventClass::Disabled);
        assert_eq!(classify_event(12), EventClass::FlagsChanged);
        assert_eq!(classify_event(10), EventClass::KeyDown);
        for other in [0, 1, 2, 11, 13, 22, 0xFFFF_FFFD] {
            assert_eq!(classify_event(other), EventClass::Ignore, "{other}");
        }
        // Key down and flags changed only: no key up (11), no mouse.
        assert_eq!(EVENT_MASK, 0x1400);
    }

    #[test]
    fn modifiers_without_fn_report_nothing() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.flags_changed(SHIFT_KEY, SHIFT, HW, 0), vec![]);
        assert_eq!(tracker.flags_changed(SHIFT_KEY, 0, HW, 10), vec![]);
        assert_eq!(tracker.key_down(20, letter()), None);
    }

    /// Every line the helper writes, exactly, as `parseFnHelperLine` in
    /// `ui/desktop/src/utils/fnKeyHelper.ts` reads them. None carries a key code.
    #[test]
    fn the_held_line_and_ready_v2_are_the_lines_the_app_parses() {
        assert_eq!(
            signal_line(Signal::FnDown, 1200),
            r#"{"type":"fn","down":true,"t":1200}"#
        );
        assert_eq!(
            signal_line(Signal::FnUp, 1500),
            r#"{"type":"fn","down":false,"t":1500}"#
        );
        assert_eq!(
            signal_line(Signal::Held, 1500),
            r#"{"type":"held","t":1500}"#
        );
        assert_eq!(signal_line(Signal::Other, 7), r#"{"type":"other","t":7}"#);
        assert_eq!(
            ready_line(Permission::Granted, true, Some(2), 900),
            r#"{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":2,"t":900}"#
        );
        assert_eq!(
            ready_line(Permission::Denied, false, None, 0),
            r#"{"type":"ready","v":2,"permission":"denied","listening":false,"fnUsage":null,"t":0}"#
        );
        assert_eq!(
            ready_line(Permission::Granted, true, Some(9), 1),
            r#"{"type":"ready","v":2,"permission":"granted","listening":true,"fnUsage":null,"t":1}"#
        );
        assert_eq!(
            permission_line(Permission::Granted),
            r#"{"type":"permission","permission":"granted"}"#
        );
        assert_eq!(LISTENER_LOST_LINE, r#"{"type":"listener-lost"}"#);
        // The held line is timed at its threshold: down plus FN_HOLD_MS.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_200);
        let at = tracker.held_due(1_600).expect("held is due");
        assert_eq!(signal_line(Signal::Held, at), r#"{"type":"held","t":1500}"#);
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
