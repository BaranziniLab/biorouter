//! What the helper tells the desktop app, and how it reads the keyboard into it.
//!
//! The helper sees every key press on the Mac while it listens, so this module is where its
//! privacy is decided. Only these facts about the keyboard ever leave the process: fn went
//! down, and which one of Shift, Control or Option was held with it, or none ([`Mod`]); fn
//! came up; fn is still down [`FN_HOLD_MS`] after it went down with nothing else pressed
//! (`held`); one of those three modifiers joined fn in its first [`FN_CHORD_GRACE_MS`]
//! (`chord`); and some other key or modifier was pressed while fn was down or within
//! [`TAP_FOLLOW_MS`] of its release (`other`). While fn is held, every other key press is
//! reported as `other` with its time; after a release only the first real key counts (one
//! from the keyboard itself, not fn or the globe key, not a repeat, and among the modifiers
//! only Command), so a tap followed by typing is not read as half of a double-tap, a key
//! that macOS or another app makes up never ends one, and a modifier pressed again between
//! the two taps of a modifier double-tap does not either. To tell a real key from the rest,
//! the tap reads a key's code, its repeat flag and its source ([`KeyFacts`]); the tracker
//! judges them on the spot and keeps none. No key code, repeat flag, source or character is
//! ever written, so a key's identity never leaves the process (a modifier is named only as
//! the one held with fn), and outside an fn press no key's timing leaves beyond that first
//! one. `held` says only how long fn itself was down.
//!
//! Separately, and with no keyboard event in it, the helper reports which other apps hold an
//! enabled event tap on modifier keys (`listeners`): the bundle identifiers of apps that are
//! not Apple's, read from macOS's list of taps ([`listener_bundles`]). That list asks for no
//! permission and says nothing about what any app reads; no other app's input ever leaves
//! this process, because this process never sees it.
//!
//! The app reads one JSON object per line (`ui/desktop/src/utils/fnKeyHelper.ts`,
//! `parseFnHelperLine`), and writes one command per line back.

/// `kCGEventFlagMaskSecondaryFn`: set while fn (the globe key) is held.
pub const SECONDARY_FN: u64 = 0x0080_0000;

/// `kCGEventFlagMaskShift`, `kCGEventFlagMaskControl`, `kCGEventFlagMaskAlternate` (Option)
/// and `kCGEventFlagMaskCommand`: the device-independent masks, set while either key of the
/// pair is down.
pub const SHIFT_FLAG: u64 = 0x0002_0000;
pub const CONTROL_FLAG: u64 = 0x0004_0000;
pub const OPTION_FLAG: u64 = 0x0008_0000;
pub const COMMAND_FLAG: u64 = 0x0010_0000;

/// Shift, Control, Option and Command (`kCGEventFlagMask*`). Caps Lock and the
/// numeric pad flag are left out: neither is a modifier a person holds with fn.
pub const OTHER_MODIFIERS: u64 = SHIFT_FLAG | CONTROL_FLAG | OPTION_FLAG | COMMAND_FLAG;
const _: () = assert!(OTHER_MODIFIERS == 0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x0010_0000);

/// The one modifier a person may hold with fn to dictate with a modifier added (Option
/// plus fn by default since the owner's ruling O-49 of 2026-11-27, `DEFAULT_FN_MODIFIER` in
/// `ui/desktop/src/utils/fnGesture.ts`; Control and Shift may be chosen). The helper treats
/// the three alike and reads no setting. Command is never one: macOS and every app use it
/// for shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mod {
    Shift,
    Control,
    Option,
}

impl Mod {
    pub const fn flag(self) -> u64 {
        match self {
            Self::Shift => SHIFT_FLAG,
            Self::Control => CONTROL_FLAG,
            Self::Option => OPTION_FLAG,
        }
    }

    /// The word a line carries for it.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Shift => "shift",
            Self::Control => "control",
            Self::Option => "option",
        }
    }

    /// The modifier a set of held modifiers ([`OTHER_MODIFIERS`] bits) is, when it is
    /// exactly one of Shift, Control or Option. None for no modifier, for Command, and for
    /// two or more.
    pub fn of_set(modifiers: u64) -> Option<Self> {
        match modifiers & OTHER_MODIFIERS {
            SHIFT_FLAG => Some(Self::Shift),
            CONTROL_FLAG => Some(Self::Control),
            OPTION_FLAG => Some(Self::Option),
            _ => None,
        }
    }
}

/// How long after a bare fn goes down one of Shift, Control or Option may still join it and
/// make it a modifier press (`chord`), as a person who presses the two together but not quite
/// at once does. Later, a modifier is another key. `FN_CHORD_GRACE_MS` in
/// `ui/desktop/src/utils/fnGesture.ts` is the same number; the helper times it, on the
/// events' own clock.
pub const FN_CHORD_GRACE_MS: u64 = 100;

/// `kVK_Function`, and the code some recent Apple keyboards report for the
/// globe key.
pub const FN_KEY_CODES: [i64; 2] = [63, 179];

/// Version 2 added the `held` line and the narrower `other` after a release. Version 3 adds
/// the modifier held with fn (`mod` on fn down, the `chord` line, `held` for a modifier
/// press, and Shift, Control and Option no longer `other` after a release) and the
/// `listeners` line. The app refuses a ready line of any other version, so a helper and an
/// app always agree on what `other` and `held` mean.
pub const PROTOCOL_VERSION: u32 = 3;

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
    /// fn went down, with the one of Shift, Control or Option held with it ([`Mod`]), or none.
    FnDown(Option<Mod>),
    FnUp,
    /// One of Shift, Control or Option joined a press that went down bare, within
    /// [`FN_CHORD_GRACE_MS`]: the press is now that modifier's.
    Chord(Mod),
    /// fn has stayed down [`FN_HOLD_MS`] with nothing else pressed but the press's own
    /// modifier, if it has one. Written with the time the threshold was reached (the press's
    /// down time plus [`FN_HOLD_MS`]).
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
    /// Read the event taps on modifier keys again and answer with a `listeners` line.
    Listeners,
    Quit,
}

pub fn parse_command(line: &str) -> Option<Command> {
    match line.trim() {
        "check" => Some(Command::Check),
        "request" => Some(Command::Request),
        "quit" => Some(Command::Quit),
        "listeners" => Some(Command::Listeners),
        _ => None,
    }
}

/// Follows fn through the event stream. It holds only whether fn is down, which other
/// modifiers are, which one of Shift, Control or Option the press now down belongs to, when
/// fn was last released until the next key comes, and when the press still waiting for
/// `held` went down, never a key code or character.
///
/// The rules, R1 to R8 of DD-101 (`docs/history/local-dictation-2026-11/implementation-plan.md`),
/// are each in one place: `press` (R1 to R3), `modifier_added_while_down` (R4), the release of
/// a modifier while fn is down (R5, nothing), `other` (R6, and R7 for a key),
/// `modifier_added_after_release` (R7 for a modifier) and [`Self::held_due`] (R8).
#[derive(Debug, Default)]
pub struct FnTracker {
    fn_down: bool,
    modifiers: u64,
    /// When fn was last released, until the first real key or modifier after it.
    released_at: Option<u64>,
    /// When the press now down went down, while it may still become a hold: nothing
    /// else was pressed with it and `held` has not been given for it yet.
    hold_from: Option<u64>,
    /// The modifier the press now down was pressed with, or that joined it in its grace.
    press_mod: Option<Mod>,
    /// While a press that went down bare may still become a modifier press: until its down
    /// time plus [`FN_CHORD_GRACE_MS`], unspoiled and before `held`.
    chord_until: Option<u64>,
}

impl FnTracker {
    pub const fn new() -> Self {
        Self {
            fn_down: false,
            modifiers: 0,
            released_at: None,
            hold_from: None,
            press_mod: None,
            chord_until: None,
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
        let added = modifiers & !self.modifiers;
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
            return self.press(modifiers, at_ms);
        }
        if self.fn_down && !fn_now {
            // The fn release was not seen (secure input took it, for one), but
            // the flags say fn is up now.
            self.fn_down = false;
            self.release(at_ms);
            return vec![Signal::FnUp];
        }
        if added == 0 {
            // R5: a modifier let go, the press's own included, ends nothing.
            return Vec::new();
        }
        let signal = if self.fn_down {
            self.modifier_added_while_down(added, modifiers, at_ms)
        } else {
            self.modifier_added_after_release(added, source, at_ms)
        };
        signal.into_iter().collect()
    }

    /// fn went down at `at_ms` with `modifiers` already held.
    fn press(&mut self, modifiers: u64, at_ms: u64) -> Vec<Signal> {
        if modifiers == 0 {
            // R1: a bare press, which one of the three may still join.
            self.hold_from = Some(at_ms);
            self.press_mod = None;
            self.chord_until = Some(at_ms.saturating_add(FN_CHORD_GRACE_MS));
            vec![Signal::FnDown(None)]
        } else if let Some(modifier) = Mod::of_set(modifiers) {
            // R2: exactly one of Shift, Control or Option, pressed before fn however long
            // before, makes the press that modifier's.
            self.hold_from = Some(at_ms);
            self.press_mod = Some(modifier);
            self.chord_until = None;
            vec![Signal::FnDown(Some(modifier))]
        } else {
            // R3: fn with Command, or with two or more modifiers, is a shortcut of its own,
            // never a dictation gesture.
            self.hold_from = None;
            self.press_mod = None;
            self.chord_until = None;
            vec![Signal::FnDown(None), Signal::Other]
        }
    }

    /// R4: `added` joined the modifiers while fn is down, which are now `modifiers`.
    fn modifier_added_while_down(
        &mut self,
        added: u64,
        modifiers: u64,
        at_ms: u64,
    ) -> Option<Signal> {
        let in_grace = self.press_mod.is_none()
            && self.hold_from.is_some()
            && self.chord_until.is_some_and(|until| at_ms <= until);
        if in_grace {
            if let Some(modifier) = Mod::of_set(modifiers) {
                // R4a: the bare press becomes this modifier's; its hold is still timed
                // from fn's own down.
                self.press_mod = Some(modifier);
                self.chord_until = None;
                return Some(Signal::Chord(modifier));
            }
        }
        if self.press_mod.is_some_and(|own| added == own.flag()) {
            // R4b: the press's own modifier pressed again after it was let go.
            return None;
        }
        // R4c: a modifier after the grace, a second one, a different one, or Command.
        self.spoil();
        Some(Signal::Other)
    }

    /// R7 for a modifier: within [`TAP_FOLLOW_MS`] of a release only Command from the
    /// keyboard ends the wait for a second tap. Shift, Control and Option type nothing, so
    /// leaving the wait open for them hides no typing, and lets a modifier double-tap have
    /// its modifier pressed again between the taps.
    fn modifier_added_after_release(
        &mut self,
        added: u64,
        source: Source,
        at_ms: u64,
    ) -> Option<Signal> {
        if added & COMMAND_FLAG == 0 {
            return None;
        }
        self.other(at_ms, source == Source::Hardware)
    }

    /// A `kCGEventKeyDown` event at `at_ms`. While fn is down, any key but fn or the
    /// globe key itself is another key, from any source and repeats included (R6); after a
    /// release only a real one is ([`KeyFacts`], R7). The facts are judged here and kept
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
    /// its threshold has passed (R8), for a bare press and a modifier press alike. `None`
    /// before the threshold, for a press already released or spoiled, and after it was
    /// given once.
    pub fn held_due(&mut self, now_ms: u64) -> Option<u64> {
        let deadline = self.hold_deadline()?;
        if now_ms < deadline {
            return None;
        }
        self.hold_from = None;
        self.chord_until = None;
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
        *self = Self::new();
        was_down.then_some(Signal::FnUp)
    }

    fn release(&mut self, at_ms: u64) {
        self.released_at = Some(at_ms);
        self.hold_from = None;
        self.press_mod = None;
        self.chord_until = None;
    }

    /// The press now down is fn used as a modifier: no hold, no chord.
    fn spoil(&mut self) {
        self.hold_from = None;
        self.chord_until = None;
    }

    /// Another key or modifier: reported while fn is down, which spoils the press, and
    /// once within [`TAP_FOLLOW_MS`] of its release when `counts_after_release`. Such a
    /// key after a release ends that window, so at most one key's timing leaves per
    /// release; a key that does not count leaves the window open for one that does.
    fn other(&mut self, at_ms: u64, counts_after_release: bool) -> Option<Signal> {
        if self.fn_down {
            self.spoil();
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
        Signal::FnDown(modifier) => format!(
            r#"{{"type":"fn","down":true,"mod":{},"t":{at_ms}}}"#,
            match modifier {
                Some(modifier) => format!(r#""{}""#, modifier.word()),
                None => "null".to_string(),
            }
        ),
        Signal::FnUp => format!(r#"{{"type":"fn","down":false,"t":{at_ms}}}"#),
        Signal::Chord(modifier) => {
            format!(
                r#"{{"type":"chord","mod":"{}","t":{at_ms}}}"#,
                modifier.word()
            )
        }
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

/// The longest line the app reads (`MAX_LINE` in `ui/desktop/src/utils/fnKeyHelper.ts`).
/// Every line here fits; [`listeners_line`] leaves out what would not.
pub const MAX_LINE: usize = 4096;

/// The most apps a `listeners` line names.
pub const MAX_LISTENER_APPS: usize = 16;

/// The longest bundle identifier a `listeners` line names.
pub const MAX_BUNDLE_ID: usize = 255;

/// How often the helper reads the list of event taps again, on a thread of its own.
pub const LISTENERS_POLL: std::time::Duration = std::time::Duration::from_secs(10);

/// `CGEventMaskBit(kCGEventFlagsChanged)`: the only event that carries fn.
pub const FLAGS_CHANGED_BIT: u64 = 1u64 << EVENT_FLAGS_CHANGED;

/// `CGEventTapInformation`, as `CGGetEventTapList` fills it (CoreGraphics'
/// `CGEventTypes.h`). The layout is pinned by `the_tap_information_layout_is_48_bytes`,
/// measured against the SDK. `enabled` is C's one-byte `bool`, read as a byte so no value
/// macOS writes can be an invalid Rust `bool`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGEventTapInformation {
    pub event_tap_id: u32,
    pub tap_point: u32,
    pub options: u32,
    pub events_of_interest: u64,
    pub tapping_process: i32,
    pub process_being_tapped: i32,
    pub enabled: u8,
    pub min_usec_latency: f32,
    pub avg_usec_latency: f32,
    pub max_usec_latency: f32,
}
const _: () = assert!(std::mem::size_of::<CGEventTapInformation>() == 48);

/// One tap macOS listed, with what `mac.rs` read about the process that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerTap {
    pub enabled: bool,
    pub events_of_interest: u64,
    pub pid: i32,
    /// The process's executable (`proc_pidpath`), when it could be read.
    pub path: Option<String>,
    /// The identifier of the outermost `.app` in `path`, when it has one.
    pub bundle: Option<String>,
}

/// Whether a tap counts at all: it is enabled, it asks for flags changed, and it is not this
/// process's own. `mac.rs` reads a process's path and bundle only for a tap that counts.
pub fn counts_as_listener(enabled: bool, events_of_interest: u64, pid: i32, own_pid: i32) -> bool {
    enabled && events_of_interest & FLAGS_CHANGED_BIT != 0 && pid != own_pid
}

/// The outermost `.app` folder in an executable's path, which names the app it belongs to
/// (a helper app inside an app belongs to the outer one). None for a path with no app in it,
/// a relative path, and a component named only `.app`.
pub fn outermost_app(path: &str) -> Option<&str> {
    if !path.starts_with('/') {
        return None;
    }
    let mut component_start = 0;
    for (slash, _) in path.match_indices('/') {
        let component = path.get(component_start..slash)?;
        if component.len() > ".app".len() && component.ends_with(".app") {
            return path.get(..slash);
        }
        component_start = slash + 1;
    }
    None
}

/// Whether a process is part of macOS: an Apple bundle, or a program in a system folder.
/// Siri holds such a tap on every Mac, and none of them acts on fn for dictation.
pub fn is_system_process(path: &str, bundle: &str) -> bool {
    bundle.starts_with("com.apple.")
        || ["/System/", "/usr/", "/Library/Apple/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

/// A bundle identifier a line may carry: 1 to [`MAX_BUNDLE_ID`] bytes of letters, digits,
/// dots and hyphens, so nothing in it needs escaping and nothing else reaches the app.
pub fn valid_bundle_id(bundle: &str) -> bool {
    !bundle.is_empty()
        && bundle.len() <= MAX_BUNDLE_ID
        && bundle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

/// The apps a `listeners` line names: the bundle of every tap that counts
/// ([`counts_as_listener`]), leaving out a process with no readable path or no bundle, a part
/// of macOS ([`is_system_process`]), the app this helper sits in (`own_bundle`), and an
/// identifier that is not one ([`valid_bundle_id`]). Sorted, each once, at most
/// [`MAX_LISTENER_APPS`].
pub fn listener_bundles(
    taps: &[ListenerTap],
    own_pid: i32,
    own_bundle: Option<&str>,
) -> Vec<String> {
    let named: std::collections::BTreeSet<&str> = taps
        .iter()
        .filter(|tap| counts_as_listener(tap.enabled, tap.events_of_interest, tap.pid, own_pid))
        .filter_map(|tap| Some((tap.path.as_deref()?, tap.bundle.as_deref()?)))
        .filter(|(path, bundle)| !is_system_process(path, bundle))
        .map(|(_, bundle)| bundle)
        .filter(|bundle| Some(*bundle) != own_bundle && valid_bundle_id(bundle))
        .collect();
    named
        .into_iter()
        .take(MAX_LISTENER_APPS)
        .map(str::to_string)
        .collect()
}

/// `{"type":"listeners","apps":[...],"t":<ms>}`. Only identifiers [`valid_bundle_id`]
/// accepts are written, at most [`MAX_LISTENER_APPS`], and no more than fit in [`MAX_LINE`].
pub fn listeners_line(bundles: &[String], at_ms: u64) -> String {
    let tail = format!(r#"],"t":{at_ms}}}"#);
    let mut line = String::from(r#"{"type":"listeners","apps":["#);
    let named = bundles
        .iter()
        .filter(|bundle| valid_bundle_id(bundle))
        .take(MAX_LISTENER_APPS);
    for (written, bundle) in named.enumerate() {
        let comma = usize::from(written > 0);
        if line.len() + comma + bundle.len() + 2 + tail.len() > MAX_LINE {
            break;
        }
        if comma == 1 {
            line.push(',');
        }
        line.push('"');
        line.push_str(bundle);
        line.push('"');
    }
    line.push_str(&tail);
    line
}

/// Whether to write a `listeners` line for the set read `now`: always when it was asked for
/// (after ready, and on the command), and otherwise only when it differs from the `last`
/// one written.
pub fn listeners_to_write(last: Option<&[String]>, now: &[String], asked: bool) -> bool {
    asked || last != Some(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u64 = 0x0002_0000;
    const CONTROL: u64 = 0x0004_0000;
    const OPTION: u64 = 0x0008_0000;
    const COMMAND: u64 = 0x0010_0000;
    /// `kVK_Shift`, `kVK_Control`, `kVK_Option` and `kVK_Command`.
    const SHIFT_KEY: i64 = 56;
    const CONTROL_KEY: i64 = 59;
    const OPTION_KEY: i64 = 58;
    const COMMAND_KEY: i64 = 55;
    /// The three modifiers a person may dictate with, with their key and flag.
    const CHORD_KEYS: [(i64, u64, Mod); 3] = [
        (SHIFT_KEY, SHIFT, Mod::Shift),
        (CONTROL_KEY, CONTROL, Mod::Control),
        (OPTION_KEY, OPTION, Mod::Option),
    ];
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
            vec![Signal::FnDown(None)]
        );
        assert_eq!(tracker.flags_changed(63, 0, HW, up), vec![Signal::FnUp]);
        tracker
    }

    #[test]
    fn fn_down_and_up_are_reported_once_each() {
        let mut tracker = FnTracker::new();
        assert_eq!(
            tracker.flags_changed(63, SECONDARY_FN, HW, 0),
            vec![Signal::FnDown(None)]
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
            vec![Signal::FnDown(None)]
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

        // Rule change in protocol version 3 (R7): Shift, Control and Option after a release no
        // longer end the wait, so a chord double-tap survives the modifier pressed again
        // between taps. Command from the keyboard still does, once, as Shift did in version 2.
        tracker.flags_changed(63, SECONDARY_FN, HW, 3_000);
        tracker.flags_changed(63, 0, HW, 3_080);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 3_100),
            vec![Signal::Other]
        );
        assert_eq!(tracker.key_down(3_120, letter()), None);
        tracker.flags_changed(COMMAND_KEY, 0, HW, 3_150);

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
                vec![Signal::FnDown(None)]
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

        // Rule change in protocol version 3 (R7): a modifier from the keyboard after a release
        // ends the wait only when it is Command; this step used Shift in version 2.
        let mut tracker = tapped(2_000, 2_090);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 2_100),
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
        // A modifier added from any source spoils it too. Rule change in protocol version 3
        // (R4a): inside the chord grace one of Shift, Control or Option makes a chord, so this
        // step adds Shift after the grace (it was 50 ms in version 2), and Command inside it.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(
            tracker.flags_changed(
                SHIFT_KEY,
                SECONDARY_FN | SHIFT,
                MADE_UP,
                1_000 + FN_CHORD_GRACE_MS + 1
            ),
            vec![Signal::Other]
        );
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, SECONDARY_FN | COMMAND, MADE_UP, 1_050),
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

    /// Rewritten from `a_modifier_pressed_while_fn_is_down_is_other_and_its_release_is_not`:
    /// its Shift came 10 ms after fn, which protocol version 3 reads as a chord (R4a). A
    /// modifier after the grace is still another key (R4c), and its release says nothing.
    #[test]
    fn a_modifier_after_the_grace_is_other() {
        for (code, flag) in [
            (SHIFT_KEY, SHIFT),
            (CONTROL_KEY, CONTROL),
            (OPTION_KEY, OPTION),
        ] {
            let mut tracker = FnTracker::new();
            tracker.flags_changed(63, SECONDARY_FN, HW, 0);
            assert_eq!(
                tracker.flags_changed(code, SECONDARY_FN | flag, HW, FN_CHORD_GRACE_MS + 1),
                vec![Signal::Other]
            );
            assert_eq!(tracker.hold_deadline(), None);
            assert_eq!(tracker.flags_changed(code, SECONDARY_FN, HW, 200), vec![]);
            assert_eq!(tracker.held_due(FN_HOLD_MS), None);
            assert_eq!(tracker.flags_changed(63, 0, HW, 400), vec![Signal::FnUp]);
        }
    }

    #[test]
    fn fn_pressed_while_command_is_held_is_down_then_other() {
        let mut tracker = FnTracker::new();
        assert_eq!(tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 0), vec![]);
        assert_eq!(
            tracker.flags_changed(63, COMMAND | SECONDARY_FN, HW, 10),
            vec![Signal::FnDown(None), Signal::Other]
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
            vec![Signal::FnDown(None)]
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
    /// `ui/desktop/src/utils/fnKeyHelper.ts` reads them. None carries a key code. Rewritten
    /// from `the_held_line_and_ready_v2_are_the_lines_the_app_parses` for protocol version 3:
    /// the ready line says 3, fn down carries `mod`, and the `chord` and `listeners` lines are
    /// new; the held, other, permission and listener-lost lines are unchanged.
    #[test]
    fn the_fn_down_chord_and_listeners_lines_are_the_lines_the_app_parses() {
        assert_eq!(PROTOCOL_VERSION, 3);
        assert_eq!(
            signal_line(Signal::FnDown(None), 1200),
            r#"{"type":"fn","down":true,"mod":null,"t":1200}"#
        );
        for (modifier, word) in [
            (Mod::Shift, "shift"),
            (Mod::Control, "control"),
            (Mod::Option, "option"),
        ] {
            assert_eq!(
                signal_line(Signal::FnDown(Some(modifier)), 1200),
                format!(r#"{{"type":"fn","down":true,"mod":"{word}","t":1200}}"#)
            );
            assert_eq!(
                signal_line(Signal::Chord(modifier), 1250),
                format!(r#"{{"type":"chord","mod":"{word}","t":1250}}"#)
            );
        }
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
            r#"{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":2,"t":900}"#
        );
        assert_eq!(
            ready_line(Permission::Denied, false, None, 0),
            r#"{"type":"ready","v":3,"permission":"denied","listening":false,"fnUsage":null,"t":0}"#
        );
        assert_eq!(
            ready_line(Permission::Granted, true, Some(9), 1),
            r#"{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":null,"t":1}"#
        );
        assert_eq!(
            permission_line(Permission::Granted),
            r#"{"type":"permission","permission":"granted"}"#
        );
        assert_eq!(LISTENER_LOST_LINE, r#"{"type":"listener-lost"}"#);
        assert_eq!(
            listeners_line(&["com.electron.wispr-flow".to_string()], 42),
            r#"{"type":"listeners","apps":["com.electron.wispr-flow"],"t":42}"#
        );
        assert_eq!(
            listeners_line(&[], 43),
            r#"{"type":"listeners","apps":[],"t":43}"#
        );
        assert_eq!(
            listeners_line(&["a.b".to_string(), "c-d.e".to_string()], 44),
            r#"{"type":"listeners","apps":["a.b","c-d.e"],"t":44}"#
        );
        // An identifier that is not one is never written, whatever the caller hands over:
        // no quote, backslash or control character can reach the app's parser.
        assert_eq!(
            listeners_line(&["a\"b".to_string(), "ok.app".to_string()], 45),
            r#"{"type":"listeners","apps":["ok.app"],"t":45}"#
        );
        // The longest listeners line still fits the app's line limit.
        let longest: Vec<String> = (0..MAX_LISTENER_APPS + 4)
            .map(|i| format!("{i:02}{}", "x".repeat(MAX_BUNDLE_ID - 2)))
            .collect();
        let line = listeners_line(&longest, u64::MAX);
        assert!(line.len() <= MAX_LINE, "{}", line.len());
        let parsed: serde_like::Line = serde_like::parse(&line);
        assert!(parsed.apps > 0 && parsed.apps <= MAX_LISTENER_APPS);
        // The held line is timed at its threshold: down plus FN_HOLD_MS.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_200);
        let at = tracker.held_due(1_600).expect("held is due");
        assert_eq!(signal_line(Signal::Held, at), r#"{"type":"held","t":1500}"#);
        // Every line is one line, inside the app's limit.
        for line in [
            signal_line(Signal::FnDown(Some(Mod::Option)), u64::MAX),
            signal_line(Signal::Chord(Mod::Control), u64::MAX),
            ready_line(Permission::Granted, true, Some(3), u64::MAX),
        ] {
            assert!(!line.contains('\n') && line.len() <= MAX_LINE);
        }
    }

    /// Just enough of a reader to count a listeners line's apps without a JSON crate: the
    /// helper links nothing but system frameworks.
    mod serde_like {
        pub struct Line {
            pub apps: usize,
        }
        pub fn parse(line: &str) -> Line {
            let inside = line
                .split_once('[')
                .and_then(|(_, rest)| rest.split_once(']'))
                .map(|(inside, _)| inside)
                .expect("an apps list");
            Line {
                apps: if inside.is_empty() {
                    0
                } else {
                    inside.split(',').count()
                },
            }
        }
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

    // ── Protocol version 3: fn with a modifier added (DD-101) ─────────────────────────

    /// R2: fn pressed while exactly one of Shift, Control or Option is held, however long
    /// before, is that modifier's press, and it is held like a bare one.
    #[test]
    fn fn_with_one_held_modifier_is_a_chord_press_with_a_hold() {
        for (code, flag, modifier) in CHORD_KEYS {
            let mut tracker = FnTracker::new();
            assert_eq!(tracker.flags_changed(code, flag, HW, 0), vec![]);
            assert_eq!(
                tracker.flags_changed(63, flag | SECONDARY_FN, HW, 5_000),
                vec![Signal::FnDown(Some(modifier))],
                "{modifier:?}"
            );
            assert_eq!(tracker.hold_deadline(), Some(5_000 + FN_HOLD_MS));
            assert_eq!(tracker.held_due(5_000 + FN_HOLD_MS - 1), None);
            assert_eq!(
                tracker.held_due(5_000 + FN_HOLD_MS),
                Some(5_000 + FN_HOLD_MS)
            );
            assert_eq!(
                tracker.flags_changed(63, flag, HW, 5_600),
                vec![Signal::FnUp]
            );
        }
    }

    /// R3: fn pressed with Command, or with two or more modifiers, is a shortcut of its own,
    /// as in version 2: fn down with no modifier, then `other`, and no hold.
    #[test]
    fn fn_with_command_or_two_modifiers_is_still_down_then_other() {
        for held in [
            COMMAND,
            SHIFT | CONTROL,
            CONTROL | OPTION,
            SHIFT | OPTION,
            CONTROL | COMMAND,
            SHIFT | CONTROL | OPTION,
        ] {
            let mut tracker = FnTracker::new();
            tracker.flags_changed(COMMAND_KEY, held, HW, 0);
            assert_eq!(
                tracker.flags_changed(63, held | SECONDARY_FN, HW, 10),
                vec![Signal::FnDown(None), Signal::Other],
                "{held:#x}"
            );
            assert_eq!(tracker.hold_deadline(), None, "{held:#x}");
            assert_eq!(tracker.held_due(10 + FN_HOLD_MS), None, "{held:#x}");
        }
    }

    /// R4a: one of the three joining a bare press inside the grace, the grace's last
    /// millisecond included, makes it that modifier's press; the hold is still timed from
    /// fn's own down.
    #[test]
    fn a_modifier_inside_the_grace_makes_a_chord_and_keeps_the_hold_time() {
        for after in [1, 50, FN_CHORD_GRACE_MS] {
            for (code, flag, modifier) in CHORD_KEYS {
                let mut tracker = FnTracker::new();
                assert_eq!(
                    tracker.flags_changed(63, SECONDARY_FN, HW, 1_000),
                    vec![Signal::FnDown(None)]
                );
                assert_eq!(
                    tracker.flags_changed(code, SECONDARY_FN | flag, HW, 1_000 + after),
                    vec![Signal::Chord(modifier)],
                    "{modifier:?} {after}"
                );
                assert_eq!(tracker.hold_deadline(), Some(1_000 + FN_HOLD_MS));
                assert_eq!(
                    tracker.held_due(1_000 + FN_HOLD_MS),
                    Some(1_000 + FN_HOLD_MS)
                );
                assert_eq!(
                    tracker.flags_changed(63, flag, HW, 1_500),
                    vec![Signal::FnUp]
                );
            }
        }
    }

    /// R4c: anything else added to a modifier press spoils it: a second modifier, a
    /// different one after its own was let go, Command, and a second modifier inside the
    /// grace of a chord.
    #[test]
    fn a_second_modifier_or_command_spoils_a_chord_press() {
        // Control held before fn, then Shift, or Command.
        for (code, flag) in [
            (SHIFT_KEY, SHIFT),
            (COMMAND_KEY, COMMAND),
            (OPTION_KEY, OPTION),
        ] {
            let mut tracker = FnTracker::new();
            tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 0);
            tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 100);
            assert_eq!(
                tracker.flags_changed(code, CONTROL | SECONDARY_FN | flag, HW, 150),
                vec![Signal::Other],
                "{flag:#x}"
            );
            assert_eq!(tracker.hold_deadline(), None);
            assert_eq!(tracker.held_due(100 + FN_HOLD_MS), None);
        }
        // Control let go, then Option: a different modifier.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 0);
        tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 100);
        assert_eq!(
            tracker.flags_changed(CONTROL_KEY, SECONDARY_FN, HW, 150),
            vec![]
        );
        assert_eq!(
            tracker.flags_changed(OPTION_KEY, SECONDARY_FN | OPTION, HW, 160),
            vec![Signal::Other]
        );
        assert_eq!(tracker.held_due(100 + FN_HOLD_MS), None);
        // A chord made inside the grace, then a second modifier inside it too.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
        assert_eq!(
            tracker.flags_changed(CONTROL_KEY, SECONDARY_FN | CONTROL, HW, 1_030),
            vec![Signal::Chord(Mod::Control)]
        );
        assert_eq!(
            tracker.flags_changed(OPTION_KEY, SECONDARY_FN | CONTROL | OPTION, HW, 1_060),
            vec![Signal::Other]
        );
        assert_eq!(tracker.held_due(1_000 + FN_HOLD_MS), None);
        // A bare press already spoiled by a letter is not saved by a modifier.
        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 2_000);
        assert_eq!(tracker.key_down(2_010, letter()), Some(Signal::Other));
        assert_eq!(
            tracker.flags_changed(CONTROL_KEY, SECONDARY_FN | CONTROL, HW, 2_020),
            vec![Signal::Other]
        );
        assert_eq!(tracker.held_due(2_000 + FN_HOLD_MS), None);
    }

    /// R5: letting go of the press's own modifier, before or after `held`, ends nothing: the
    /// hold goes on until fn is released.
    #[test]
    fn letting_go_of_the_chord_modifier_keeps_the_hold() {
        for (code, flag, _) in CHORD_KEYS {
            // Pressed before fn, let go before the threshold.
            let mut tracker = FnTracker::new();
            tracker.flags_changed(code, flag, HW, 0);
            tracker.flags_changed(63, flag | SECONDARY_FN, HW, 100);
            assert_eq!(tracker.flags_changed(code, SECONDARY_FN, HW, 200), vec![]);
            assert_eq!(tracker.hold_deadline(), Some(100 + FN_HOLD_MS));
            assert_eq!(tracker.held_due(100 + FN_HOLD_MS), Some(100 + FN_HOLD_MS));
            assert_eq!(tracker.flags_changed(63, 0, HW, 900), vec![Signal::FnUp]);

            // Joined inside the grace, let go after `held`.
            let mut tracker = FnTracker::new();
            tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
            tracker.flags_changed(code, SECONDARY_FN | flag, HW, 1_040);
            assert_eq!(
                tracker.held_due(1_000 + FN_HOLD_MS),
                Some(1_000 + FN_HOLD_MS)
            );
            assert_eq!(tracker.flags_changed(code, SECONDARY_FN, HW, 1_500), vec![]);
            assert_eq!(tracker.flags_changed(63, 0, HW, 2_000), vec![Signal::FnUp]);
        }
    }

    /// R4b: the press's own modifier pressed again after it was let go, while fn is still
    /// down, is nothing.
    #[test]
    fn pressing_the_chord_modifier_again_while_fn_is_down_is_nothing() {
        for (code, flag, _) in CHORD_KEYS {
            let mut tracker = FnTracker::new();
            tracker.flags_changed(code, flag, HW, 0);
            tracker.flags_changed(63, flag | SECONDARY_FN, HW, 100);
            assert_eq!(tracker.flags_changed(code, SECONDARY_FN, HW, 150), vec![]);
            assert_eq!(
                tracker.flags_changed(code, SECONDARY_FN | flag, HW, 250),
                vec![]
            );
            assert_eq!(tracker.held_due(100 + FN_HOLD_MS), Some(100 + FN_HOLD_MS));
            // After the grace of a chord made by joining, too.
            let mut tracker = FnTracker::new();
            tracker.flags_changed(63, SECONDARY_FN, HW, 1_000);
            tracker.flags_changed(code, SECONDARY_FN | flag, HW, 1_020);
            assert_eq!(tracker.flags_changed(code, SECONDARY_FN, HW, 1_080), vec![]);
            assert_eq!(
                tracker.flags_changed(code, SECONDARY_FN | flag, HW, 1_200),
                vec![]
            );
            assert_eq!(
                tracker.held_due(1_000 + FN_HOLD_MS),
                Some(1_000 + FN_HOLD_MS)
            );
        }
    }

    /// R8: `held` is given once, at the threshold, for a press with a modifier, whichever
    /// way the modifier came, and its line is the plain `held` line.
    #[test]
    fn held_is_given_for_a_chord_press() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 0);
        tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 2_000);
        // A timer that fires late still writes the threshold's time, once.
        let at = tracker.held_due(2_450).expect("held is due");
        assert_eq!(at, 2_000 + FN_HOLD_MS);
        assert_eq!(signal_line(Signal::Held, at), r#"{"type":"held","t":2300}"#);
        assert_eq!(tracker.held_due(2_500), None);
        assert_eq!(tracker.hold_deadline(), None);

        let mut tracker = FnTracker::new();
        tracker.flags_changed(63, SECONDARY_FN, HW, 3_000);
        tracker.flags_changed(OPTION_KEY, SECONDARY_FN | OPTION, HW, 3_090);
        assert_eq!(tracker.held_due(3_300), Some(3_300));
        assert_eq!(tracker.held_due(3_301), None);
        // A modifier added after `held` is another key, as in version 2.
        assert_eq!(
            tracker.flags_changed(SHIFT_KEY, SECONDARY_FN | OPTION | SHIFT, HW, 3_400),
            vec![Signal::Other]
        );
    }

    /// R7: within the double-tap gap after a release, Shift, Control and Option from the
    /// keyboard end nothing and leave the wait open, so the next letter still counts; Command
    /// from the keyboard and a letter each end the wait, once. A Command a program makes up
    /// ends nothing, as in version 2.
    #[test]
    fn after_a_release_shift_control_and_option_do_not_end_the_wait_but_command_and_a_letter_do() {
        let mut tracker = tapped(1_000, 1_080);
        for (at, (code, flag, _)) in [1_100, 1_120, 1_140].into_iter().zip(CHORD_KEYS) {
            assert_eq!(
                tracker.flags_changed(code, flag, HW, at),
                vec![],
                "{flag:#x}"
            );
            assert_eq!(
                tracker.flags_changed(code, 0, HW, at + 5),
                vec![],
                "{flag:#x}"
            );
        }
        assert_eq!(tracker.key_down(1_200, letter()), Some(Signal::Other));
        assert_eq!(tracker.key_down(1_210, letter()), None);

        let mut tracker = tapped(2_000, 2_080);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, MADE_UP, 2_090),
            vec![]
        );
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, 0, MADE_UP, 2_095),
            vec![]
        );
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 2_100),
            vec![Signal::Other]
        );
        assert_eq!(tracker.flags_changed(COMMAND_KEY, 0, HW, 2_110), vec![]);
        // The window is spent: neither a letter nor Command counts again.
        assert_eq!(tracker.key_down(2_150, letter()), None);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 2_160),
            vec![]
        );

        // Command with a chord modifier already held still counts.
        let mut tracker = tapped(3_000, 3_080);
        tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 3_090);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, CONTROL | COMMAND, HW, 3_100),
            vec![Signal::Other]
        );
        // Past the gap nothing counts.
        let mut tracker = tapped(4_000, 4_080);
        assert_eq!(
            tracker.flags_changed(COMMAND_KEY, COMMAND, HW, 4_080 + TAP_FOLLOW_MS + 1),
            vec![]
        );
    }

    /// A Control plus fn double-tap is two taps with no `other` between them, whether
    /// Control stays down between the taps or is let go and pressed again, and a bare tap
    /// followed by a Control plus fn tap is too.
    #[test]
    fn a_chord_double_tap_with_the_modifier_let_go_between_taps_is_two_taps() {
        // Let go and pressed again between the taps.
        let mut tracker = FnTracker::new();
        let mut lines = Vec::new();
        lines.extend(tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 0));
        lines.extend(tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 100));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 180));
        lines.extend(tracker.flags_changed(CONTROL_KEY, 0, HW, 200));
        lines.extend(tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 300));
        lines.extend(tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 350));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 420));
        lines.extend(tracker.flags_changed(CONTROL_KEY, 0, HW, 450));
        assert_eq!(
            lines,
            vec![
                Signal::FnDown(Some(Mod::Control)),
                Signal::FnUp,
                Signal::FnDown(Some(Mod::Control)),
                Signal::FnUp,
            ]
        );
        assert_eq!(tracker.held_due(LATER), None, "two taps, no hold");

        // Control held throughout.
        let mut tracker = FnTracker::new();
        let mut lines = Vec::new();
        lines.extend(tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 0));
        lines.extend(tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 100));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 170));
        lines.extend(tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 300));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 380));
        assert_eq!(
            lines,
            vec![
                Signal::FnDown(Some(Mod::Control)),
                Signal::FnUp,
                Signal::FnDown(Some(Mod::Control)),
                Signal::FnUp,
            ]
        );

        // A bare tap, then Control, then a Control plus fn tap.
        let mut tracker = tapped(1_000, 1_080);
        let mut lines = Vec::new();
        lines.extend(tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 1_150));
        lines.extend(tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 1_200));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 1_270));
        assert_eq!(
            lines,
            vec![Signal::FnDown(Some(Mod::Control)), Signal::FnUp]
        );

        // Joined inside the grace on each tap.
        let mut tracker = FnTracker::new();
        let mut lines = Vec::new();
        lines.extend(tracker.flags_changed(63, SECONDARY_FN, HW, 5_000));
        lines.extend(tracker.flags_changed(CONTROL_KEY, SECONDARY_FN | CONTROL, HW, 5_030));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 5_090));
        lines.extend(tracker.flags_changed(CONTROL_KEY, 0, HW, 5_100));
        lines.extend(tracker.flags_changed(63, SECONDARY_FN, HW, 5_200));
        lines.extend(tracker.flags_changed(CONTROL_KEY, SECONDARY_FN | CONTROL, HW, 5_220));
        lines.extend(tracker.flags_changed(63, CONTROL, HW, 5_290));
        assert_eq!(
            lines,
            vec![
                Signal::FnDown(None),
                Signal::Chord(Mod::Control),
                Signal::FnUp,
                Signal::FnDown(None),
                Signal::Chord(Mod::Control),
                Signal::FnUp,
            ]
        );
    }

    /// A press's modifier never carries over: the next bare press is bare, and a blind gap
    /// forgets the grace.
    #[test]
    fn a_press_starts_with_no_modifier_of_its_own() {
        let mut tracker = FnTracker::new();
        tracker.flags_changed(CONTROL_KEY, CONTROL, HW, 0);
        tracker.flags_changed(63, CONTROL | SECONDARY_FN, HW, 10);
        tracker.flags_changed(63, CONTROL, HW, 50);
        tracker.flags_changed(CONTROL_KEY, 0, HW, 60);
        assert_eq!(
            tracker.flags_changed(63, SECONDARY_FN, HW, LATER),
            vec![Signal::FnDown(None)]
        );
        assert_eq!(tracker.reset_after_blind_gap(), Some(Signal::FnUp));
        // A modifier right after the reset joins nothing.
        assert_eq!(
            tracker.flags_changed(CONTROL_KEY, CONTROL, HW, LATER + 10),
            vec![]
        );
    }

    // ── Collision detection: who else taps modifier keys ──────────────────────────────

    /// `CGEventTapInformation` as CoreGraphics lays it out, measured with clang against the
    /// macOS SDK (`sizeof` and `offsetof` of the C struct, 2026-11-27): 48 bytes, 8-byte
    /// aligned.
    #[test]
    fn the_tap_information_layout_is_48_bytes() {
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!(size_of::<CGEventTapInformation>(), 48);
        assert_eq!(align_of::<CGEventTapInformation>(), 8);
        assert_eq!(
            [
                offset_of!(CGEventTapInformation, event_tap_id),
                offset_of!(CGEventTapInformation, tap_point),
                offset_of!(CGEventTapInformation, options),
                offset_of!(CGEventTapInformation, events_of_interest),
                offset_of!(CGEventTapInformation, tapping_process),
                offset_of!(CGEventTapInformation, process_being_tapped),
                offset_of!(CGEventTapInformation, enabled),
                offset_of!(CGEventTapInformation, min_usec_latency),
                offset_of!(CGEventTapInformation, avg_usec_latency),
                offset_of!(CGEventTapInformation, max_usec_latency),
            ],
            [0, 4, 8, 16, 24, 28, 32, 36, 40, 44]
        );
    }

    const OWN_PID: i32 = 4_242;
    const WISPR: &str = "/Applications/Wispr Flow.app/Contents/MacOS/Wispr Flow";
    /// flagsChanged and key down, as most hotkey apps ask.
    const KEYS_MASK: u64 = (1 << 12) | (1 << 10);

    fn tap(pid: i32, path: Option<&str>, bundle: Option<&str>) -> ListenerTap {
        ListenerTap {
            enabled: true,
            events_of_interest: KEYS_MASK,
            pid,
            path: path.map(str::to_string),
            bundle: bundle.map(str::to_string),
        }
    }

    #[test]
    fn listener_bundles_drop_apple_system_unbundled_own_and_disabled_taps() {
        let taps = vec![
            tap(100, Some(WISPR), Some("com.electron.wispr-flow")),
            // Siri, as on every Mac.
            tap(
                101,
                Some("/System/Applications/Siri.app/Contents/MacOS/Siri"),
                Some("com.apple.Siri"),
            ),
            // An Apple bundle outside /System.
            tap(
                102,
                Some("/Applications/Xcode.app/Contents/MacOS/Xcode"),
                Some("com.apple.dt.Xcode"),
            ),
            // System processes with no bundle, and one whose path could not be read.
            tap(103, Some("/usr/libexec/universalaccessd"), None),
            tap(104, None, None),
            tap(
                105,
                Some("/Library/Apple/System/Library/Thing.app/Contents/MacOS/Thing"),
                Some("org.example.thing"),
            ),
            tap(
                106,
                Some("/System/Library/CoreServices/Other.app/Contents/MacOS/Other"),
                Some("org.example.other"),
            ),
            // A third-party process with no bundle.
            tap(107, Some("/opt/homebrew/bin/skhd"), None),
            // This helper itself, and another process of the app it sits in.
            tap(
                OWN_PID,
                Some("/Applications/Biorouter.app/Contents/Resources/bin/biorouter-fnkey"),
                Some("org.biorouter.desktop"),
            ),
            tap(
                108,
                Some("/Applications/Biorouter.app/Contents/MacOS/Biorouter"),
                Some("org.biorouter.desktop"),
            ),
            // A disabled tap, as Logi Options+ held one.
            ListenerTap {
                enabled: false,
                ..tap(
                    109,
                    Some("/Applications/Logi Options+.app/Contents/MacOS/x"),
                    Some("com.logi.cp-dev-mgr"),
                )
            },
        ];
        assert_eq!(
            listener_bundles(&taps, OWN_PID, Some("org.biorouter.desktop")),
            vec!["com.electron.wispr-flow".to_string()]
        );
        // Without a bundle of its own (a dev build), the app's other process counts.
        assert_eq!(
            listener_bundles(&taps, OWN_PID, None),
            vec![
                "com.electron.wispr-flow".to_string(),
                "org.biorouter.desktop".to_string()
            ]
        );
        assert!(is_system_process("/System/x", "org.example"));
        assert!(is_system_process("/usr/sbin/x", "org.example"));
        assert!(is_system_process("/Library/Apple/x", "org.example"));
        assert!(is_system_process("/Applications/X.app/x", "com.apple.X"));
        assert!(!is_system_process(
            "/Applications/X.app/x",
            "com.applesauce.X"
        ));
        assert!(!is_system_process(
            "/Library/Application Support/x",
            "org.example"
        ));
        assert!(!is_system_process("/Users/me/System/x", "org.example"));
    }

    #[test]
    fn count_only_taps_on_flags_changed() {
        assert!(counts_as_listener(true, 1 << 12, 100, OWN_PID));
        assert!(counts_as_listener(true, KEYS_MASK, 100, OWN_PID));
        assert!(counts_as_listener(true, u64::MAX, 100, OWN_PID));
        // Key down alone, mouse events, nothing.
        assert!(!counts_as_listener(true, 1 << 10, 100, OWN_PID));
        assert!(!counts_as_listener(
            true,
            (1 << 1) | (1 << 2) | (1 << 5),
            100,
            OWN_PID
        ));
        assert!(!counts_as_listener(true, 0, 100, OWN_PID));
        // Disabled, or this process's own tap.
        assert!(!counts_as_listener(false, 1 << 12, 100, OWN_PID));
        assert!(!counts_as_listener(true, 1 << 12, OWN_PID, OWN_PID));

        let mouse_only = ListenerTap {
            events_of_interest: (1 << 1) | (1 << 2),
            ..tap(100, Some(WISPR), Some("com.electron.wispr-flow"))
        };
        let keys_only = ListenerTap {
            events_of_interest: 1 << 10,
            ..tap(
                101,
                Some("/Applications/Keys.app/Contents/MacOS/Keys"),
                Some("org.example.keys"),
            )
        };
        let flags = ListenerTap {
            events_of_interest: 1 << 12,
            ..tap(
                102,
                Some("/Applications/Flags.app/Contents/MacOS/Flags"),
                Some("org.example.flags"),
            )
        };
        assert_eq!(
            listener_bundles(&[mouse_only, keys_only, flags], OWN_PID, None),
            vec!["org.example.flags".to_string()]
        );
    }

    #[test]
    fn the_outermost_app_names_the_bundle() {
        // A helper app inside an app: the outer one is the bundle.
        assert_eq!(
            outermost_app(
                "/Applications/Wispr Flow.app/Contents/Frameworks/Wispr Flow Helper.app/Contents/MacOS/Wispr Flow Helper"
            ),
            Some("/Applications/Wispr Flow.app")
        );
        assert_eq!(outermost_app(WISPR), Some("/Applications/Wispr Flow.app"));
        assert_eq!(
            outermost_app("/Users/me/Applications/Tool.app/Contents/MacOS/tool"),
            Some("/Users/me/Applications/Tool.app")
        );
        // No app in the path, a name that only starts like one, or an app with no name.
        assert_eq!(outermost_app("/opt/homebrew/bin/skhd"), None);
        assert_eq!(outermost_app("/Applications/My.application/bin/x"), None);
        assert_eq!(outermost_app("/Applications/X.app"), None);
        assert_eq!(outermost_app("/weird/.app/Contents/MacOS/x"), None);
        assert_eq!(outermost_app(""), None);
        assert_eq!(outermost_app("relative/Thing.app/Contents/MacOS/x"), None);
    }

    #[test]
    fn listener_bundles_are_checked_sorted_deduplicated_and_capped_at_16() {
        let mut taps = vec![
            tap(
                1,
                Some("/Applications/B.app/Contents/MacOS/b"),
                Some("org.example.b"),
            ),
            tap(
                2,
                Some("/Applications/A.app/Contents/MacOS/a"),
                Some("org.example.a"),
            ),
            // The same app twice (two taps, or two processes).
            tap(
                3,
                Some("/Applications/A.app/Contents/MacOS/a"),
                Some("org.example.a"),
            ),
            // Not identifiers: a space, a quote, a slash, empty, too long, not ASCII.
            tap(
                4,
                Some("/Applications/C.app/Contents/MacOS/c"),
                Some("org.example c"),
            ),
            tap(
                5,
                Some("/Applications/D.app/Contents/MacOS/d"),
                Some("org.\"example"),
            ),
            tap(
                6,
                Some("/Applications/E.app/Contents/MacOS/e"),
                Some("org/example"),
            ),
            tap(7, Some("/Applications/F.app/Contents/MacOS/f"), Some("")),
            tap(
                8,
                Some("/Applications/G.app/Contents/MacOS/g"),
                Some(&"g".repeat(MAX_BUNDLE_ID + 1)),
            ),
            tap(
                9,
                Some("/Applications/H.app/Contents/MacOS/h"),
                Some("org.exämple"),
            ),
        ];
        assert_eq!(
            listener_bundles(&taps, OWN_PID, None),
            vec!["org.example.a".to_string(), "org.example.b".to_string()]
        );
        assert!(valid_bundle_id(&"g".repeat(MAX_BUNDLE_ID)));
        assert!(valid_bundle_id("com.electron.wispr-flow"));
        assert!(valid_bundle_id("ABC.def-9"));
        for bad in ["", "a b", "a_b", "a\nb", "a\"b", "a\\b", "ä"] {
            assert!(!valid_bundle_id(bad), "{bad:?}");
        }

        // Twenty distinct apps: the first sixteen by name.
        taps.clear();
        for i in (0..20).rev() {
            taps.push(tap(
                100 + i,
                Some("/Applications/X.app/Contents/MacOS/x"),
                Some(&format!("org.example.app{i:02}")),
            ));
        }
        let listed = listener_bundles(&taps, OWN_PID, None);
        assert_eq!(listed.len(), MAX_LISTENER_APPS);
        let want: Vec<String> = (0..16).map(|i| format!("org.example.app{i:02}")).collect();
        assert_eq!(listed, want);
    }

    /// A listeners line goes out after ready and on the command whatever it says, and
    /// otherwise only when the set changed.
    #[test]
    fn a_listeners_line_is_written_when_asked_or_when_the_set_changes() {
        let wispr = vec!["com.electron.wispr-flow".to_string()];
        assert!(
            listeners_to_write(None, &[], false),
            "the first one, after ready"
        );
        assert!(listeners_to_write(None, &wispr, false));
        assert!(!listeners_to_write(Some(&wispr), &wispr, false));
        assert!(listeners_to_write(Some(&wispr), &wispr, true), "asked");
        assert!(listeners_to_write(Some(&wispr), &[], false));
        assert!(listeners_to_write(Some(&[]), &wispr, false));
        assert!(!listeners_to_write(Some(&[]), &[], false));
    }

    #[test]
    fn listeners_is_an_exact_command_word() {
        assert_eq!(parse_command("listeners"), Some(Command::Listeners));
        assert_eq!(parse_command(" listeners\n"), Some(Command::Listeners));
        assert_eq!(parse_command("LISTENERS"), None);
        assert_eq!(parse_command("listeners now"), None);
        assert_eq!(parse_command("listener"), None);
    }
}
