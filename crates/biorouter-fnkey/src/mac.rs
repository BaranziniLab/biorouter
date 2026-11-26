//! The Quartz side: Input Monitoring, the listen-only event tap, the hold timer on the
//! tap's run loop, and what macOS itself does with fn. Declared by hand against the
//! system frameworks, so the helper links nothing but them.

use std::cell::Cell;
use std::ffi::{c_char, c_void, CString};
use std::io::BufRead;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::outbox::{LineKind, SharedOutbox, CAPACITY};
use crate::protocol::{
    classify_event, event_ms, parse_command, permission_line, ready_line, signal_line, Command,
    EventClass, FnTracker, KeyFacts, Permission, Signal, Source, EVENT_MASK, LISTENER_LOST_LINE,
};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventTapCallBack =
    extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;
type CFRunLoopTimerRef = *mut c_void;
type CFAbsoluteTime = f64;
type CFRunLoopTimerCallBack = extern "C" fn(CFRunLoopTimerRef, *mut c_void);

const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const TAP_OPTION_LISTEN_ONLY: u32 = 1;
/// `CGEventField`s read from a key event, all judged inside this process and never written.
/// `kCGKeyboardEventAutorepeat`: non-zero when the key down is a repeat.
const KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
/// `kCGKeyboardEventKeycode`.
const KEYBOARD_EVENT_KEYCODE: u32 = 9;
/// `kCGEventSourceStateID`: which event source state made the event; the keyboard's own
/// events carry `kCGEventSourceStateHIDSystemState`.
const EVENT_SOURCE_STATE_ID: u32 = 45;
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT64_TYPE: isize = 4;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetTimestamp(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

extern "C" {
    fn clock_gettime_nsec_np(clock: u32) -> u64;
}

/// `CLOCK_UPTIME_RAW`: the clock Quartz stamps key events with (nanoseconds since start-up,
/// not counting sleep), so the ready line's time and the events' times compare.
const CLOCK_UPTIME_RAW: u32 = 8;

fn now_ms() -> u64 {
    // SAFETY: a plain read of a system clock.
    unsafe { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1_000_000 }
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: CFTypeRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFAbsoluteTimeGetCurrent() -> CFAbsoluteTime;
    fn CFRunLoopTimerCreate(
        allocator: CFTypeRef,
        fire_date: CFAbsoluteTime,
        interval: f64,
        flags: usize,
        order: isize,
        callout: CFRunLoopTimerCallBack,
        context: *mut c_void,
    ) -> CFRunLoopTimerRef;
    fn CFRunLoopAddTimer(run_loop: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);
    fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
    fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        text: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFPreferencesCopyAppValue(key: CFStringRef, application: CFStringRef) -> CFTypeRef;
    fn CFGetTypeID(value: CFTypeRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> bool;
    fn CFRelease(value: CFTypeRef);
}

static TRACKER: Mutex<FnTracker> = Mutex::new(FnTracker::new());
static TAP: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
/// Every line to the app goes through here, so they reach it in the order they were made.
static OUTBOX: SharedOutbox = SharedOutbox::new(CAPACITY);

thread_local! {
    /// The hold timer while one is armed, with the threshold it is armed for (the press's down
    /// time plus `FN_HOLD_MS`, on the events' clock). At most one exists. Only the run loop's
    /// thread touches it: the tap's callback and the timer's both run there.
    static HOLD_TIMER: Cell<Option<(CFRunLoopTimerRef, u64)>> = const { Cell::new(None) };
}

/// How long a leaving process waits for its queued lines to reach the app.
const EXIT_FLUSH: Duration = Duration::from_millis(250);

/// One line to the app, queued for the writer thread. It never waits on the pipe, so the
/// tap's callback, which emits too, stays fast enough that macOS keeps the tap on.
fn emit(kind: LineKind, line: String) {
    OUTBOX.push(kind, line);
}

fn emit_signal(signal: Signal, at_ms: u64) {
    emit(LineKind::of(signal), signal_line(signal, at_ms));
}

/// The only thread that writes to standard output. A closed pipe means the app is gone, so
/// this process goes too.
fn write_to_app() {
    OUTBOX.write_lines(&mut std::io::stdout());
    std::process::exit(0);
}

/// Leave, after giving the queued lines a moment to reach the app.
fn exit_after_flush() -> ! {
    OUTBOX.flush_for(EXIT_FLUSH);
    std::process::exit(0);
}

fn permission() -> Permission {
    // SAFETY: a plain query of this process's Input Monitoring; it never prompts.
    if unsafe { CGPreflightListenEventAccess() } {
        Permission::Granted
    } else {
        Permission::Denied
    }
}

fn cf_string(text: &str) -> CFStringRef {
    let text = CString::new(text).expect("a constant without NUL");
    // SAFETY: `text` is a valid NUL-terminated string for the duration of the call.
    unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), CF_STRING_ENCODING_UTF8) }
}

/// macOS's own use of fn (System Settings > Keyboard > "Press fn key to"), when set.
fn fn_usage() -> Option<i64> {
    let key = cf_string("AppleFnUsageType");
    let domain = cf_string("com.apple.HIToolbox");
    if key.is_null() || domain.is_null() {
        for created in [key, domain] {
            if !created.is_null() {
                // SAFETY: created by `cf_string` above (the Create rule) and not used again.
                unsafe { CFRelease(created) };
            }
        }
        return None;
    }
    // SAFETY: both strings were created above and are released below; the value is
    // checked for its type before it is read, and released after.
    unsafe {
        let value = CFPreferencesCopyAppValue(key, domain);
        CFRelease(key);
        CFRelease(domain);
        if value.is_null() {
            return None;
        }
        let mut number: i64 = 0;
        let read = CFGetTypeID(value) == CFNumberGetTypeID()
            && CFNumberGetValue(
                value,
                CF_NUMBER_SINT64_TYPE,
                (&mut number as *mut i64).cast::<c_void>(),
            );
        CFRelease(value);
        read.then_some(number)
    }
}

/// Run one event through the tracker, write what it reports at `at_ms`, and keep the hold
/// timer in step with the press it waits on.
fn track(at_ms: u64, step: impl FnOnce(&mut FnTracker) -> Vec<Signal>) {
    let (signals, wanted) = match TRACKER.lock() {
        Ok(mut tracker) => (step(&mut tracker), tracker.hold_deadline()),
        Err(_) => return,
    };
    for signal in signals {
        emit_signal(signal, at_ms);
    }
    follow_hold(wanted);
}

/// Bring the hold timer in line with what the tracker waits for: no timer, or one armed for
/// `wanted`. Runs on the run loop's thread only.
fn follow_hold(wanted: Option<u64>) {
    let armed = HOLD_TIMER.with(Cell::get).map(|(_, deadline)| deadline);
    if armed == wanted {
        return;
    }
    cancel_hold_timer();
    if let Some(deadline) = wanted {
        arm_hold_timer(deadline);
    }
}

fn cancel_hold_timer() {
    if let Some((timer, _)) = HOLD_TIMER.with(Cell::take) {
        // SAFETY: `timer` was created by `arm_hold_timer` (the Create rule) and is held only by
        // `HOLD_TIMER`, which no longer names it; invalidating a fired timer again is harmless.
        unsafe {
            CFRunLoopTimerInvalidate(timer);
            CFRelease(timer.cast_const());
        }
    }
}

/// A one-shot timer on this thread's run loop, the tap's, for `deadline` on the events' clock.
/// A threshold already passed is given at once.
fn arm_hold_timer(deadline: u64) {
    let now = now_ms();
    if deadline <= now {
        give_held(now);
        return;
    }
    let wait = Duration::from_millis(deadline - now).as_secs_f64();
    // SAFETY: a plain timer with no context; the callback is a plain function. The timer is
    // added to the run loop of the thread this runs on, the one that runs the tap.
    let timer = unsafe {
        CFRunLoopTimerCreate(
            std::ptr::null(),
            CFAbsoluteTimeGetCurrent() + wait,
            0.0,
            0,
            0,
            on_hold_timer,
            null_mut(),
        )
    };
    if timer.is_null() {
        // No timer, no `held`: the app still reads a release after the threshold as a hold.
        return;
    }
    // SAFETY: `timer` is the non-NULL timer created above, and this is the tap's thread.
    unsafe { CFRunLoopAddTimer(CFRunLoopGetCurrent(), timer, kCFRunLoopCommonModes) };
    HOLD_TIMER.with(|cell| cell.set(Some((timer, deadline))));
}

/// The threshold has passed by the clock now: write `held` if the press still waits for it.
/// Should a timer ever fire a moment early, the press still waits, and it is armed again.
fn give_held(now: u64) {
    let (held, wanted) = match TRACKER.lock() {
        Ok(mut tracker) => (tracker.held_due(now), tracker.hold_deadline()),
        Err(_) => return,
    };
    if let Some(at_ms) = held {
        emit_signal(Signal::Held, at_ms);
    }
    follow_hold(wanted);
}

extern "C" fn on_hold_timer(timer: CFRunLoopTimerRef, _info: *mut c_void) {
    // A one-shot timer is spent once it fires, and the run loop invalidates it after this call:
    // forget it, so the next press arms a new one.
    HOLD_TIMER.with(|cell| {
        if cell.get().is_some_and(|(armed, _)| armed == timer) {
            cell.set(None);
            // SAFETY: ours (the Create rule) and named nowhere else now; the run loop keeps its
            // own reference while it calls this function.
            unsafe { CFRelease(timer.cast_const()) };
        }
    });
    give_held(now_ms());
}

/// macOS turned the tap off. Turn it on again, and end a hold its blind gap may have hidden
/// the release of.
fn on_tap_disabled() {
    let tap = TAP.load(Ordering::Acquire);
    if tap.is_null() {
        return;
    }
    // SAFETY: `tap` is the port this process created; it is never freed.
    unsafe { CGEventTapEnable(tap, true) };
    // The tap was blind while it was off and may have missed fn's release, which
    // would leave the app holding fn until some later modifier event.
    track(now_ms(), |tracker| {
        tracker.reset_after_blind_gap().into_iter().collect()
    });
    // SAFETY: as above.
    if !unsafe { CGEventTapIsEnabled(tap) } {
        emit(LineKind::ListenerLost, LISTENER_LOST_LINE.to_string());
    }
}

extern "C" fn on_event(
    _proxy: CGEventTapProxy,
    kind: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    match classify_event(kind) {
        // `event` is NULL for these: nothing reads it.
        EventClass::Disabled => on_tap_disabled(),
        EventClass::FlagsChanged => {
            // SAFETY: macOS hands the callback a valid event for its duration.
            let (key_code, flags, state_id, stamp) = unsafe {
                (
                    CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE),
                    CGEventGetFlags(event),
                    CGEventGetIntegerValueField(event, EVENT_SOURCE_STATE_ID),
                    CGEventGetTimestamp(event),
                )
            };
            let at_ms = event_ms(stamp, now_ms());
            let source = Source::of_state(state_id);
            track(at_ms, |tracker| {
                tracker.flags_changed(key_code, flags, source, at_ms)
            });
        }
        EventClass::KeyDown => {
            // SAFETY: as above. The facts stay in this process: the tracker judges them and
            // keeps none, and no line carries them.
            let (facts, stamp) = unsafe {
                (
                    KeyFacts {
                        key_code: CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE),
                        autorepeat: CGEventGetIntegerValueField(event, KEYBOARD_EVENT_AUTOREPEAT)
                            != 0,
                        source: Source::of_state(CGEventGetIntegerValueField(
                            event,
                            EVENT_SOURCE_STATE_ID,
                        )),
                    },
                    CGEventGetTimestamp(event),
                )
            };
            let at_ms = event_ms(stamp, now_ms());
            track(at_ms, |tracker| {
                tracker.key_down(at_ms, facts).into_iter().collect()
            });
        }
        EventClass::Ignore => {}
    }
    // A listen-only tap's return value is ignored; the event passes unchanged.
    event
}

/// Commands from the app, until its end of the pipe closes.
fn serve_commands() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        match parse_command(&line) {
            Some(Command::Check) => emit(LineKind::Permission, permission_line(permission())),
            Some(Command::Request) => {
                // SAFETY: shows the system prompt the first time; later calls return at once.
                unsafe { CGRequestListenEventAccess() };
                emit(LineKind::Permission, permission_line(permission()));
            }
            Some(Command::Quit) => break,
            None => {}
        }
    }
    exit_after_flush();
}

fn wait_for_commands() -> ! {
    loop {
        std::thread::park();
    }
}

pub fn run() {
    std::thread::spawn(write_to_app);
    let usage = fn_usage();
    std::thread::spawn(serve_commands);

    if permission() == Permission::Denied {
        emit(
            LineKind::Ready,
            ready_line(Permission::Denied, false, usage, now_ms()),
        );
        wait_for_commands();
    }

    // SAFETY: Input Monitoring was granted just above, so this does not prompt; the
    // callback is a plain function and the tap lives for the whole process.
    let tap = unsafe {
        CGEventTapCreate(
            SESSION_EVENT_TAP,
            HEAD_INSERT_EVENT_TAP,
            TAP_OPTION_LISTEN_ONLY,
            EVENT_MASK,
            on_event,
            null_mut(),
        )
    };
    if tap.is_null() {
        emit(
            LineKind::Ready,
            ready_line(Permission::Granted, false, usage, now_ms()),
        );
        wait_for_commands();
    }
    // SAFETY: `tap` is the non-NULL port created above.
    let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };
    if source.is_null() {
        // No source, no events: turn the tap off and give it back, and say the listener
        // is not running, as for a tap that could not be created.
        // SAFETY: `tap` was created above (the Create rule), nothing else holds it, and
        // the callback never saw it, because `TAP` is still NULL.
        unsafe {
            CGEventTapEnable(tap, false);
            CFRelease(tap.cast_const());
        }
        emit(
            LineKind::Ready,
            ready_line(Permission::Granted, false, usage, now_ms()),
        );
        wait_for_commands();
    }
    TAP.store(tap, Ordering::Release);
    // SAFETY: the source and the run loop belong to this thread; nothing is freed
    // while the process runs.
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    emit(
        LineKind::Ready,
        ready_line(Permission::Granted, true, usage, now_ms()),
    );
    // SAFETY: runs this thread's run loop, which delivers the tap's events.
    unsafe { CFRunLoopRun() };
}
