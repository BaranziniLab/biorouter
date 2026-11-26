//! The macOS side: the app in front, secure input, Accessibility, the keyboard layout, the
//! Command-V, and the threads that serve the app.
//!
//! Three threads. The main thread runs its run loop and the short pieces of work handed to it,
//! so what AppKit reports about the app in front stays current (NSWorkspace learns of a change
//! through that run loop), and the keyboard layout and the pasteboard are read where AppKit and
//! the Text Input Sources API want them. A reader thread answers `front` and the access requests
//! at once. One worker thread pastes, one insert at a time, and waits (for the modifiers, for
//! the restore) off the main thread, so a `front` asked while an insert is under way is not
//! kept waiting.

use std::ffi::c_void;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use objc2::rc::{autoreleasepool, Retained};
use objc2_app_kit::{NSPasteboard, NSWorkspace};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGPreflightPostEventAccess,
    CGRequestPostEventAccess,
};
use objc2_foundation::NSString;

use crate::pasteboard;
use crate::protocol::{
    access_line, front_line, insert_refusal, inserted_line, modifiers_held, parse_args,
    parse_request, past_deadline, paste_flags, ready_line, recover_id, refused_line,
    reported_bundle, reported_pid, v_key_code, FrontFacts, InsertFacts, LineReader, Next, Op,
    Options, Parsed, Refusal, Request, Text, ANSI_V_KEY_CODE, MAX_REQUEST_LINE, MODIFIER_POLL,
    MODIFIER_WAIT, RESTORE_DELAY,
};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type TISInputSourceRef = *mut c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> TISInputSourceRef;
    fn TISGetInputSourceProperty(source: TISInputSourceRef, key: CFStringRef) -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    fn LMGetKbdType() -> u8;
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    #[allow(clippy::too_many_arguments)]
    fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: u32,
        dead_key_state: *mut u32,
        max_string_length: usize,
        actual_string_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
    fn CFRelease(value: CFTypeRef);
    fn CFRunLoopRun();
}

/// `kUCKeyActionDown`.
const KEY_ACTION_DOWN: u16 = 0;
/// `(cmdKey >> 8) & 0xFF`: Command, in the form `UCKeyTranslate` takes modifiers.
const COMMAND_KEY_STATE: u32 = 1;
/// `kUCKeyTranslateNoDeadKeysMask`: a dead key types nothing rather than waiting for the next.
const NO_DEAD_KEYS: u32 = 1;

static OPTIONS: OnceLock<Options> = OnceLock::new();
/// Set once the app has closed its end: inserts still queued are refused, never pasted.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// How long a leaving helper waits for an insert under way to restore the pasteboard.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// One answer to the app. A closed pipe means the app is gone; the reader then sees its input end
/// and the helper leaves once an insert under way has put the pasteboard back.
fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
        CLOSING.store(true, Ordering::Release);
    }
}

fn post_allowed() -> bool {
    // A plain query of this process's Accessibility; it never prompts.
    CGPreflightPostEventAccess()
}

fn secure_input() -> bool {
    // SAFETY: a plain query with no arguments.
    unsafe { IsSecureEventInputEnabled() != 0 }
}

/// Run `work` on the main thread and wait for it. Only the reader and the worker call this; the
/// main thread itself only runs its run loop, which is what serves the main queue.
fn on_main<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    let mut result = None;
    DispatchQueue::main().exec_sync(|| result = Some(work()));
    result.expect("the main queue runs the work before exec_sync returns")
}

/// The pid of the app in front. Main thread only.
fn frontmost_pid() -> Option<i32> {
    autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .and_then(|app| reported_pid(app.processIdentifier()))
    })
}

fn front_facts() -> FrontFacts {
    let (pid, bundle) = on_main(|| {
        autoreleasepool(|_| {
            let app = NSWorkspace::sharedWorkspace().frontmostApplication();
            let pid = app
                .as_ref()
                .and_then(|app| reported_pid(app.processIdentifier()));
            let bundle = app
                .and_then(|app| app.bundleIdentifier())
                .map(|bundle| bundle.to_string());
            (pid, reported_bundle(bundle))
        })
    });
    FrontFacts {
        pid,
        bundle,
        secure_input: secure_input(),
        post_allowed: post_allowed(),
    }
}

/// What `code` types in `layout`, with Command held or with no modifier.
///
/// # Safety
///
/// `layout` points to a `UCKeyboardLayout` that stays valid for the call.
unsafe fn translate(layout: *const u8, code: u16, command: bool, keyboard: u32) -> Option<String> {
    let mut dead_key_state = 0u32;
    let mut chars = [0u16; 4];
    let mut length = 0usize;
    // SAFETY: the layout is valid (the caller's promise); every out pointer is a local of the
    // size given.
    let status = unsafe {
        UCKeyTranslate(
            layout.cast(),
            code,
            KEY_ACTION_DOWN,
            if command { COMMAND_KEY_STATE } else { 0 },
            keyboard,
            NO_DEAD_KEYS,
            &mut dead_key_state,
            chars.len(),
            &mut length,
            chars.as_mut_ptr(),
        )
    };
    (status == 0 && length > 0).then(|| String::from_utf16_lossy(&chars[..length.min(chars.len())]))
}

/// The key code that types `v` in the current keyboard layout. Main thread only: the Text Input
/// Sources API asserts it is called there.
fn layout_v_key_code() -> u16 {
    // SAFETY: the source is released below and nothing read from it outlives it; the layout
    // data is owned by the source (the Get rule) and used only while the source is held.
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return ANSI_V_KEY_CODE;
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let layout = if data.is_null() {
            std::ptr::null()
        } else {
            CFDataGetBytePtr(data)
        };
        let code = if layout.is_null() {
            ANSI_V_KEY_CODE
        } else {
            let keyboard = u32::from(LMGetKbdType());
            v_key_code(|code, command| translate(layout, code, command, keyboard))
        };
        CFRelease(source.cast_const());
        code
    }
}

/// The pasteboard an insert goes through: a named one for a live test, the general one
/// otherwise. The only place the general pasteboard is named.
fn target_pasteboard() -> Retained<NSPasteboard> {
    match OPTIONS
        .get()
        .and_then(|options| options.test_pasteboard.as_deref())
    {
        Some(name) => NSPasteboard::pasteboardWithName(&NSString::from_str(name)),
        None => NSPasteboard::generalPasteboard(),
    }
}

/// The two Command-V events, from a private source (so no modifier the system thinks is down
/// joins them) with their flags set to Command alone, handed to `use_them`.
fn with_paste_events<T>(
    key_code: u16,
    use_them: impl FnOnce(&CGEvent, &CGEvent) -> T,
) -> Option<T> {
    let source = CGEventSource::new(CGEventSourceStateID::Private)?;
    let down = CGEvent::new_keyboard_event(Some(&source), key_code, true)?;
    let up = CGEvent::new_keyboard_event(Some(&source), key_code, false)?;
    let flags = CGEventFlags(paste_flags());
    CGEvent::set_flags(Some(&down), flags);
    CGEvent::set_flags(Some(&up), flags);
    Some(use_them(&down, &up))
}

/// Post Command-V to `pid` alone: if another app came to the front since the last look, the
/// paste still cannot reach it.
fn post_command_v(pid: i32, key_code: u16) -> bool {
    with_paste_events(key_code, |down, up| {
        CGEvent::post_to_pid(pid, Some(down));
        CGEvent::post_to_pid(pid, Some(up));
    })
    .is_some()
}

/// Wait, briefly, for the person to let go of the modifiers.
fn wait_for_modifiers() {
    let started = Instant::now();
    while modifiers_held(CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState).0)
        && started.elapsed() < MODIFIER_WAIT
    {
        std::thread::sleep(MODIFIER_POLL);
    }
}

/// Whether `pid` may receive the paste now.
fn check(pid: i32) -> Result<(), Refusal> {
    let post_allowed = post_allowed();
    let secure_input = secure_input();
    let front_pid = on_main(frontmost_pid);
    match insert_refusal(
        pid,
        &InsertFacts {
            post_allowed,
            secure_input,
            front_pid,
        },
    ) {
        Some(refusal) => Err(refusal),
        None => Ok(()),
    }
}

fn in_time(received: Instant) -> Result<(), Refusal> {
    if past_deadline(received.elapsed()) {
        Err(Refusal::Failed)
    } else {
        Ok(())
    }
}

/// A snapshot carried from the main thread, where it was taken, to the worker and back to the
/// main thread, where it is restored. The worker only holds it.
struct Kept(pasteboard::Snapshot);

// SAFETY: a snapshot holds only immutable `NSString` and `NSData` objects (the pasteboard types
// and the data `dataForType:` returned), which Foundation documents as safe to use from any
// thread, and retain counts are atomic. It is never read off the main thread.
unsafe impl Send for Kept {}

/// One insert: judge, keep the pasteboard, write the text, look again, paste, wait, put the
/// pasteboard back. Whatever happens after the text is written, the pasteboard is put back.
///
/// The pasteboard is read and written on the main thread: reading a type the pasteboard derives
/// (or another app promised) fulfils it synchronously, which AppKit expects there.
fn paste(pid: i32, text: &Text, received: Instant) -> Result<(), Refusal> {
    check(pid)?;
    let key_code = on_main(layout_v_key_code);
    let prepared = on_main(|| {
        autoreleasepool(|_| {
            let pasteboard = target_pasteboard();
            let kept = pasteboard::snapshot(&pasteboard);
            in_time(received)?;
            let written = pasteboard::write_transient_text(&pasteboard, text.as_str());
            Ok((Kept(kept), written))
        })
    });
    let (kept, written) = prepared?;
    let outcome = if written.ok {
        wait_for_modifiers();
        // The last look: a password field focused, or another app brought forward, while the
        // text was being prepared.
        check(pid).and_then(|()| in_time(received)).and_then(|()| {
            if post_command_v(pid, key_code) {
                Ok(())
            } else {
                Err(Refusal::Failed)
            }
        })
    } else {
        Err(Refusal::Failed)
    };
    if outcome.is_ok() {
        std::thread::sleep(RESTORE_DELAY);
    }
    on_main(move || {
        // The whole wrapper moves to the main thread, not only the snapshot inside it.
        let kept = kept;
        autoreleasepool(|_| pasteboard::restore(&target_pasteboard(), kept.0, written))
    });
    outcome
}

struct Job {
    id: u64,
    pid: i32,
    text: Text,
    received: Instant,
}

fn insert(job: Job) -> String {
    let Job {
        id,
        pid,
        text,
        received,
    } = job;
    if CLOSING.load(Ordering::Acquire) {
        return refused_line(id, Refusal::Failed);
    }
    let outcome = autoreleasepool(|_| paste(pid, &text, received));
    // Zeroed here, before the answer is written.
    drop(text);
    match outcome {
        Ok(()) => inserted_line(id),
        Err(refusal) => refused_line(id, refusal),
    }
}

/// Requests from the app, until its end of the pipe closes.
fn serve(jobs: &mpsc::Sender<Job>) {
    // SAFETY: descriptor 0 is this process's standard input, open for its whole life, and read
    // only here. Read directly, so no buffer of the standard library ever holds a request.
    let stdin = unsafe { std::fs::File::from_raw_fd(0) };
    let mut reader = LineReader::new(stdin, MAX_REQUEST_LINE);
    loop {
        let parsed = match reader.next() {
            Ok(Next::Line) => parse_request(reader.line()),
            Ok(Next::TooLong) => Parsed::Unusable {
                id: recover_id(reader.line()),
            },
            Ok(Next::End) | Err(_) => break,
        };
        reader.wipe();
        match parsed {
            Parsed::Unusable { id: Some(id) } => emit(&refused_line(id, Refusal::Failed)),
            Parsed::Unusable { id: None } => {}
            Parsed::Request(Request { id, op }) => match op {
                Op::Front => emit(&front_line(id, &front_facts())),
                Op::Access => emit(&access_line(id, post_allowed())),
                Op::RequestAccess => {
                    // Shows the system prompt the first time; later calls return at once. The
                    // app sends this only from a person's click in Settings.
                    CGRequestPostEventAccess();
                    emit(&access_line(id, post_allowed()));
                }
                Op::Insert { pid, text } => {
                    let job = Job {
                        id,
                        pid,
                        text,
                        received: Instant::now(),
                    };
                    if let Err(mpsc::SendError(job)) = jobs.send(job) {
                        emit(&refused_line(job.id, Refusal::Failed));
                    }
                }
            },
        }
    }
}

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(options) = parse_args(&args) else {
        eprintln!("usage: biorouter-textinsert [--test-pasteboard NAME]");
        std::process::exit(64);
    };
    let _ = OPTIONS.set(options);

    emit(&ready_line(post_allowed()));

    let (jobs, queue) = mpsc::channel::<Job>();
    let (finished, worker_done) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        for job in queue {
            let answer = insert(job);
            emit(&answer);
        }
        let _ = finished.send(());
    });
    std::thread::spawn(move || {
        serve(&jobs);
        CLOSING.store(true, Ordering::Release);
        drop(jobs);
        let _ = worker_done.recv_timeout(EXIT_WAIT);
        std::process::exit(0);
    });

    loop {
        // SAFETY: runs this (the main) thread's run loop, which also serves the main queue. It
        // does not return while the main queue can receive work; the pause only guards against
        // a busy loop if it ever did.
        unsafe { CFRunLoopRun() };
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_graphics::{CGEventField, CGEventType};

    #[test]
    fn the_paste_events_carry_command_only_and_the_layout_key() {
        // Made, never posted.
        for key_code in [ANSI_V_KEY_CODE, 47] {
            let read = with_paste_events(key_code, |down, up| {
                [down, up].map(|event| {
                    (
                        CGEvent::r#type(Some(event)),
                        CGEvent::flags(Some(event)).0,
                        CGEvent::integer_value_field(
                            Some(event),
                            CGEventField::KeyboardEventKeycode,
                        ),
                    )
                })
            })
            .expect("a private source makes keyboard events");
            assert_eq!(
                read,
                [
                    (CGEventType::KeyDown, 0x0010_0000, i64::from(key_code)),
                    (CGEventType::KeyUp, 0x0010_0000, i64::from(key_code)),
                ]
            );
        }
    }
}
