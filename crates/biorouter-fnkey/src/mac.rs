//! The Quartz side: Input Monitoring, the listen-only event tap, the hold timer on the
//! tap's run loop, what macOS itself does with fn, and which other apps tap modifier keys.
//! Declared by hand against the system frameworks, so the helper links nothing but them.
//!
//! Four threads. The main thread runs the tap's run loop: the tap's callback and the hold
//! timer, and nothing that can wait. One thread writes lines to the app (`outbox.rs`), one
//! reads its commands, and one watches the list of event taps ([`watch_listeners`]), which
//! the run loop never does: reading the list and the bundles behind it touches the file
//! system, and the tap's callback must stay fast enough that macOS keeps the tap on. The
//! watch creates no tap, asks for no permission and changes no setting.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::io::BufRead;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::outbox::{LineKind, SharedOutbox, CAPACITY};
use crate::protocol::{
    classify_event, counts_as_listener, event_ms, listener_bundles, listeners_line,
    listeners_to_write, outermost_app, parse_command, permission_line, ready_line, signal_line,
    CGEventTapInformation, Command, EventClass, FnTracker, KeyFacts, ListenerTap, Permission,
    Signal, Source, EVENT_MASK, LISTENERS_POLL, LISTENER_LOST_LINE, MAX_BUNDLE_ID,
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
    /// Lists the event taps in the session; with a NULL list, only how many there are. Needs
    /// no permission and creates nothing.
    fn CGGetEventTapList(
        max_number_of_taps: u32,
        tap_list: *mut CGEventTapInformation,
        event_tap_count: *mut u32,
    ) -> i32;
}

extern "C" {
    fn clock_gettime_nsec_np(clock: u32) -> u64;
    /// libproc: the executable of a process. A process of the same user needs no permission.
    fn proc_pidpath(pid: i32, buffer: *mut c_void, buffer_size: u32) -> i32;
}

/// `PROC_PIDPATHINFO_MAXSIZE`.
const PROC_PIDPATHINFO_MAXSIZE: usize = 4 * 1024;

/// The most taps read from the list; a session holds a handful.
const MAX_TAPS: u32 = 256;

/// How many apps' identifiers the watch remembers by folder before it starts afresh.
const MAX_REMEMBERED_APPS: usize = 64;

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
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: CFTypeRef,
        buffer: *const u8,
        buffer_length: isize,
        is_directory: u8,
    ) -> CFTypeRef;
    fn CFBundleCreate(allocator: CFTypeRef, bundle_url: CFTypeRef) -> CFTypeRef;
    fn CFBundleGetIdentifier(bundle: CFTypeRef) -> CFStringRef;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
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

/// The command thread's request to the listener watch for a `listeners` line now.
struct ListenerAsk {
    asked: Mutex<bool>,
    wake: Condvar,
}

static LISTENER_ASK: ListenerAsk = ListenerAsk {
    asked: Mutex::new(false),
    wake: Condvar::new(),
};

/// Ask the watch for a `listeners` line. Only sets a flag: the reading stays on the watch's
/// thread.
fn ask_for_listeners() {
    *LISTENER_ASK
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = true;
    LISTENER_ASK.wake.notify_one();
}

/// Wait up to `limit` for [`ask_for_listeners`]. `true` when it was asked.
fn wait_for_listener_ask(limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    let mut asked = LISTENER_ASK
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    while !*asked {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        asked = match LISTENER_ASK.wake.wait_timeout(asked, left) {
            Ok((asked, _)) => asked,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
    *asked = false;
    true
}

/// The taps the session holds now, at most [`MAX_TAPS`]. `None` when the list cannot be read.
fn read_tap_list() -> Option<Vec<CGEventTapInformation>> {
    let mut count: u32 = 0;
    // SAFETY: a NULL list asks only for the count, written to a local.
    if unsafe { CGGetEventTapList(0, null_mut(), &mut count) } != 0 {
        return None;
    }
    // Room for a few taps made between the two calls.
    let room = count.saturating_add(8).min(MAX_TAPS);
    let mut taps = vec![CGEventTapInformation::default(); room as usize];
    let mut filled: u32 = 0;
    // SAFETY: `taps` holds `room` elements of the layout CoreGraphics fills (pinned by
    // `the_tap_information_layout_is_48_bytes`), and the count goes to a local.
    if unsafe { CGGetEventTapList(room, taps.as_mut_ptr(), &mut filled) } != 0 {
        return None;
    }
    taps.truncate(filled.min(room) as usize);
    Some(taps)
}

/// The executable of `pid`, when it can be read as text.
fn process_path(pid: i32) -> Option<String> {
    let mut buffer = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
    // SAFETY: the buffer is as long as the size given, and proc_pidpath writes no more.
    let written = unsafe {
        proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast::<c_void>(),
            PROC_PIDPATHINFO_MAXSIZE as u32,
        )
    };
    let written = usize::try_from(written).ok().filter(|len| *len > 0)?;
    buffer.truncate(written.min(PROC_PIDPATHINFO_MAXSIZE));
    String::from_utf8(buffer).ok()
}

/// The identifier in the `Info.plist` of the app folder `app`, when it has one that fits.
fn read_bundle_identifier(app: &str) -> Option<String> {
    let bytes = app.as_bytes();
    let length = isize::try_from(bytes.len()).ok()?;
    // SAFETY: the URL and the bundle are created here (the Create rule) and released below;
    // the identifier belongs to the bundle (the Get rule) and is copied out before the bundle
    // is released; the buffer is as long as the size given.
    unsafe {
        let url =
            CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), length, 1);
        if url.is_null() {
            return None;
        }
        let bundle = CFBundleCreate(std::ptr::null(), url);
        CFRelease(url);
        if bundle.is_null() {
            return None;
        }
        let identifier = CFBundleGetIdentifier(bundle);
        let mut text = [0 as c_char; MAX_BUNDLE_ID + 1];
        let copied = !identifier.is_null()
            && CFStringGetCString(
                identifier,
                text.as_mut_ptr(),
                text.len() as isize,
                CF_STRING_ENCODING_UTF8,
            ) != 0;
        CFRelease(bundle);
        if !copied {
            return None;
        }
        CStr::from_ptr(text.as_ptr())
            .to_str()
            .ok()
            .map(str::to_string)
    }
}

/// App folders the watch has read the identifier of, so a poll reads no `Info.plist` it read
/// before. Forgotten whole once it holds [`MAX_REMEMBERED_APPS`].
#[derive(Default)]
struct AppIdentifiers(HashMap<String, Option<String>>);

impl AppIdentifiers {
    fn of(&mut self, app: &str) -> Option<String> {
        if let Some(known) = self.0.get(app) {
            return known.clone();
        }
        if self.0.len() >= MAX_REMEMBERED_APPS {
            self.0.clear();
        }
        let identifier = read_bundle_identifier(app);
        self.0.insert(app.to_string(), identifier.clone());
        identifier
    }
}

/// The apps that tap modifier keys now ([`listener_bundles`]), or `None` when macOS's list
/// cannot be read. A process's path and bundle are read only for a tap that counts.
fn read_listeners(
    own_pid: i32,
    own_bundle: Option<&str>,
    apps: &mut AppIdentifiers,
) -> Option<Vec<String>> {
    let taps = read_tap_list()?
        .iter()
        .map(|tap| {
            let enabled = tap.enabled != 0;
            let counted = counts_as_listener(
                enabled,
                tap.events_of_interest,
                tap.tapping_process,
                own_pid,
            );
            let path = if counted {
                process_path(tap.tapping_process)
            } else {
                None
            };
            let bundle = path
                .as_deref()
                .and_then(outermost_app)
                .and_then(|app| apps.of(app));
            ListenerTap {
                enabled,
                events_of_interest: tap.events_of_interest,
                pid: tap.tapping_process,
                path,
                bundle,
            }
        })
        .collect::<Vec<_>>();
    Some(listener_bundles(&taps, own_pid, own_bundle))
}

/// The listener watch, on a thread of its own and never on the tap's run loop: the list of
/// apps that tap modifier keys, written after the ready line, whenever it changes (read every
/// [`LISTENERS_POLL`]) and on the `listeners` command.
fn watch_listeners() {
    let own_pid = i32::try_from(std::process::id()).unwrap_or(-1);
    let mut apps = AppIdentifiers::default();
    let own_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.to_str().map(str::to_string));
    let own_bundle = own_exe
        .as_deref()
        .and_then(outermost_app)
        .and_then(|app| apps.of(app));
    let mut last: Option<Vec<String>> = None;
    let mut asked = true;
    loop {
        // A list that cannot be read changes nothing; asked for, it says the last one read.
        let now = read_listeners(own_pid, own_bundle.as_deref(), &mut apps)
            .or_else(|| last.clone())
            .unwrap_or_default();
        if listeners_to_write(last.as_deref(), &now, asked) {
            emit(LineKind::Listeners, listeners_line(&now, now_ms()));
            last = Some(now);
        }
        asked = wait_for_listener_ask(LISTENERS_POLL);
    }
}

/// The first line, then the listener watch, so its first line comes after the ready line. The
/// watch runs whether or not Input Monitoring is granted: reading the list asks for nothing.
fn announce_ready(permission: Permission, listening: bool, usage: Option<i64>) {
    emit(
        LineKind::Ready,
        ready_line(permission, listening, usage, now_ms()),
    );
    std::thread::spawn(watch_listeners);
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
            Some(Command::Listeners) => ask_for_listeners(),
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
        announce_ready(Permission::Denied, false, usage);
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
        announce_ready(Permission::Granted, false, usage);
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
        announce_ready(Permission::Granted, false, usage);
        wait_for_commands();
    }
    TAP.store(tap, Ordering::Release);
    // SAFETY: the source and the run loop belong to this thread; nothing is freed
    // while the process runs.
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    announce_ready(Permission::Granted, true, usage);
    // SAFETY: runs this thread's run loop, which delivers the tap's events.
    unsafe { CFRunLoopRun() };
}

#[cfg(test)]
mod tests {
    //! The system calls behind the listener watch, run for real. None of them creates a tap,
    //! asks for a permission or changes a setting: they are the reads the watch makes.

    use super::*;
    use crate::protocol::{valid_bundle_id, MAX_LISTENER_APPS};

    /// The list reads with no permission, and what the watch makes of it names only apps
    /// that are not Apple's, each once, sorted, at most sixteen. Whether any app is running
    /// is this machine's business, so nothing is asserted about which.
    #[test]
    fn the_tap_list_reads_without_a_permission_and_names_only_other_apps() {
        let taps = read_tap_list().expect("the tap list reads with no permission");
        assert!(taps.len() <= MAX_TAPS as usize);
        let own_pid = i32::try_from(std::process::id()).unwrap();
        let named = read_listeners(own_pid, None, &mut AppIdentifiers::default())
            .expect("the tap list reads");
        assert!(named.len() <= MAX_LISTENER_APPS);
        assert!(
            named.windows(2).all(|pair| pair[0] < pair[1]),
            "sorted, each once"
        );
        for bundle in &named {
            assert!(valid_bundle_id(bundle));
            assert!(!bundle.starts_with("com.apple."));
        }
        // Every tap's mask and pid came through the declared layout: a pid is never 0 or
        // negative for a listed tap.
        assert!(taps.iter().all(|tap| tap.tapping_process > 0));
    }

    #[test]
    fn a_process_path_and_an_app_identifier_read_as_the_watch_reads_them() {
        let own = process_path(i32::try_from(std::process::id()).unwrap()).expect("own path");
        assert_eq!(
            std::path::Path::new(&own).canonicalize().ok(),
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.canonicalize().ok())
        );
        assert_eq!(process_path(-1), None);
        assert_eq!(
            read_bundle_identifier("/System/Applications/Calculator.app").as_deref(),
            Some("com.apple.calculator")
        );
        assert_eq!(read_bundle_identifier("/nonexistent/Nothing.app"), None);
        let mut apps = AppIdentifiers::default();
        assert_eq!(
            apps.of("/System/Applications/Calculator.app").as_deref(),
            Some("com.apple.calculator")
        );
        assert_eq!(apps.0.len(), 1);
        assert_eq!(apps.of("/nonexistent/Nothing.app"), None);
        assert_eq!(apps.0.len(), 2, "a miss is remembered too");
    }
}
