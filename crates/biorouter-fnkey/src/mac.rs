//! The Quartz side: Input Monitoring, the listen-only event tap, and what macOS
//! itself does with fn. Declared by hand against the system frameworks, so the
//! helper links nothing but them.

use std::ffi::{c_char, c_void, CString};
use std::io::{BufRead, Write};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;

use crate::protocol::{
    classify_event, parse_command, permission_line, ready_line, signal_line, Command, EventClass,
    FnTracker, Permission, EVENT_MASK, LISTENER_LOST_LINE,
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

const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const TAP_OPTION_LISTEN_ONLY: u32 = 1;
const KEYBOARD_EVENT_KEYCODE: u32 = 9;
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

/// One line to the app. A closed pipe means the app is gone, so this process goes too.
fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
        std::process::exit(0);
    }
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

extern "C" fn on_event(
    _proxy: CGEventTapProxy,
    kind: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    match classify_event(kind) {
        // `event` is NULL for these: nothing below reads it.
        EventClass::Disabled => {
            let tap = TAP.load(Ordering::Acquire);
            if !tap.is_null() {
                // SAFETY: `tap` is the port this process created; it is never freed.
                unsafe { CGEventTapEnable(tap, true) };
                // The tap was blind while it was off and may have missed fn's release, which
                // would leave the app holding fn until some later modifier event.
                let signal = TRACKER
                    .lock()
                    .ok()
                    .and_then(|mut tracker| tracker.reset_after_blind_gap());
                if let Some(signal) = signal {
                    emit(&signal_line(signal, now_ms()));
                }
                // SAFETY: as above.
                if !unsafe { CGEventTapIsEnabled(tap) } {
                    emit(LISTENER_LOST_LINE);
                }
            }
        }
        EventClass::FlagsChanged => {
            // SAFETY: macOS hands the callback a valid event for its duration.
            let (key_code, flags, at_ms) = unsafe {
                (
                    CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE),
                    CGEventGetFlags(event),
                    CGEventGetTimestamp(event) / 1_000_000,
                )
            };
            let signals = match TRACKER.lock() {
                Ok(mut tracker) => tracker.flags_changed(key_code, flags, at_ms),
                Err(_) => Vec::new(),
            };
            for signal in signals {
                emit(&signal_line(signal, at_ms));
            }
        }
        EventClass::KeyDown => {
            // SAFETY: as above.
            let at_ms = unsafe { CGEventGetTimestamp(event) } / 1_000_000;
            let signal = TRACKER
                .lock()
                .ok()
                .and_then(|mut tracker| tracker.key_down(at_ms));
            if let Some(signal) = signal {
                emit(&signal_line(signal, at_ms));
            }
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
            Some(Command::Check) => emit(&permission_line(permission())),
            Some(Command::Request) => {
                // SAFETY: shows the system prompt the first time; later calls return at once.
                unsafe { CGRequestListenEventAccess() };
                emit(&permission_line(permission()));
            }
            Some(Command::Quit) => break,
            None => {}
        }
    }
    std::process::exit(0);
}

fn wait_for_commands() -> ! {
    loop {
        std::thread::park();
    }
}

pub fn run() {
    let usage = fn_usage();
    std::thread::spawn(serve_commands);

    if permission() == Permission::Denied {
        emit(&ready_line(Permission::Denied, false, usage, now_ms()));
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
        emit(&ready_line(Permission::Granted, false, usage, now_ms()));
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
        emit(&ready_line(Permission::Granted, false, usage, now_ms()));
        wait_for_commands();
    }
    TAP.store(tap, Ordering::Release);
    // SAFETY: the source and the run loop belong to this thread; nothing is freed
    // while the process runs.
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    emit(&ready_line(Permission::Granted, true, usage, now_ms()));
    // SAFETY: runs this thread's run loop, which delivers the tap's events.
    unsafe { CFRunLoopRun() };
}
