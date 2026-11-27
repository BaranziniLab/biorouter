//! The macOS side: the app in front, secure input, Accessibility, the keyboard layout, the
//! Command-V, and the threads that serve the app.
//!
//! Three threads. The main thread runs its run loop and the short pieces of work handed to it,
//! so what AppKit reports about the app in front stays current (NSWorkspace learns of a change
//! through that run loop), and the keyboard layout and the pasteboard are read where AppKit and
//! the Text Input Sources API want them. A reader thread answers `front` and the access requests
//! at once. One worker thread pastes, one insert at a time, and waits (for the modifiers, for
//! the restore) off the main thread, so a `front` asked while an insert is under way is not
//! kept waiting. Because the worker finishes an insert, the restore included, before it takes
//! the next, two dictations in a row each snapshot what the person copied, never the text of
//! the one before.
//!
//! What an insert needs from the system goes through [`Desk`], so the worker's own loop runs in
//! a test on a private pasteboard with a post that only records what it would have pasted.

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
    access_line, front_line, insert_refusal, inserted_line, may_post, parse_args, parse_request,
    paste_flags, ready_line, recover_id, refused_line, reported_bundle, reported_pid,
    restore_delay, v_key_code, wait_for_release, FrontFacts, InsertFacts, LineReader, Next, Op,
    Options, Parsed, Refusal, Request, Restored, Text, ANSI_V_KEY_CODE, MAX_REQUEST_LINE,
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

/// When an insert's request arrived, and when the worker began it: later, for an insert that
/// waited in the queue for the one before it to finish.
#[derive(Clone, Copy)]
struct Times {
    received: Instant,
    started: Instant,
}

/// Whether the paste of `chars` characters may still be posted at `now` ([`may_post`]).
fn in_time(now: Instant, times: Times, chars: usize) -> Result<(), Refusal> {
    if may_post(
        now.saturating_duration_since(times.received),
        now.saturating_duration_since(times.started),
        chars,
    ) {
        Ok(())
    } else {
        Err(Refusal::Failed)
    }
}

/// A snapshot carried from the main thread, where it was taken, to the worker and back to the
/// main thread, where it is restored. The worker only holds it.
struct Kept(pasteboard::Snapshot);

// SAFETY: a snapshot holds only immutable `NSString` and `NSData` objects (the pasteboard types
// and the data `dataForType:` returned), which Foundation documents as safe to use from any
// thread, and retain counts are atomic. It is never read off the main thread.
unsafe impl Send for Kept {}

/// What an insert needs from the system. [`MacDesk`] is the real one; a test's stands in with a
/// private pasteboard and a post that records.
trait Desk {
    /// Whether `pid` may receive the paste now.
    fn check(&self, pid: i32) -> Result<(), Refusal>;
    /// The key code that types `v` in the current layout.
    fn v_key_code(&self) -> u16;
    /// Run `work` with the pasteboard inserts go through, on the thread AppKit wants it on.
    fn with_pasteboard<T: Send>(&self, work: impl FnOnce(&NSPasteboard) -> T + Send) -> T;
    /// The modifiers the HID system says are down now.
    fn modifier_flags(&self) -> u64;
    /// Post Command-V to `pid`.
    fn post(&self, pid: i32, key_code: u16) -> bool;
    /// Wait this long.
    fn pause(&self, wait: Duration);
    /// The clock an insert's times are kept on, callable from the main thread too.
    fn clock(&self) -> &(dyn Fn() -> Instant + Sync);
}

/// The system itself: the main thread for AppKit, the HID system's flags, the real post.
struct MacDesk;

impl Desk for MacDesk {
    fn check(&self, pid: i32) -> Result<(), Refusal> {
        check(pid)
    }

    fn v_key_code(&self) -> u16 {
        on_main(layout_v_key_code)
    }

    fn with_pasteboard<T: Send>(&self, work: impl FnOnce(&NSPasteboard) -> T + Send) -> T {
        on_main(|| autoreleasepool(|_| work(&target_pasteboard())))
    }

    fn modifier_flags(&self) -> u64 {
        CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState).0
    }

    fn post(&self, pid: i32, key_code: u16) -> bool {
        post_command_v(pid, key_code)
    }

    fn pause(&self, wait: Duration) {
        std::thread::sleep(wait);
    }

    fn clock(&self) -> &(dyn Fn() -> Instant + Sync) {
        &Instant::now
    }
}

/// One insert: judge, keep the pasteboard, write the text, look again, paste, wait, put the
/// pasteboard back. Whatever happens after the text is written, the pasteboard is put back, and
/// an insert that pasted answers how that left it.
///
/// The pasteboard is read and written on the main thread: reading a type the pasteboard derives
/// (or another app promised) fulfils it synchronously, which AppKit expects there.
fn paste(desk: &impl Desk, pid: i32, text: &Text, times: Times) -> Result<Restored, Refusal> {
    desk.check(pid)?;
    let key_code = desk.v_key_code();
    let now = desk.clock();
    let chars = text.as_str().chars().count();
    let prepared = desk.with_pasteboard(|pasteboard| {
        let kept = pasteboard::snapshot(pasteboard);
        // A slow snapshot (another app fulfilling what it promised) can use up the time.
        in_time(now(), times, chars)?;
        let written = pasteboard::write_transient_text(pasteboard, text.as_str());
        Ok((Kept(kept), written))
    });
    let (kept, written) = prepared?;
    let outcome = if written.ok {
        // Wait, briefly, for the person to let go of the modifiers: an Option still down after
        // Option plus fn (the default) would otherwise reach the app as Option-Command-V, and a
        // Control after Control plus fn as Control-Command-V.
        let waiting = now();
        wait_for_release(
            || desk.modifier_flags(),
            || now().saturating_duration_since(waiting),
            |wait| desk.pause(wait),
        );
        // The last look: a password field focused, or another app brought forward, while the
        // text was being prepared.
        desk.check(pid)
            .and_then(|()| in_time(now(), times, chars))
            .and_then(|()| {
                if desk.post(pid, key_code) {
                    Ok(())
                } else {
                    Err(Refusal::Failed)
                }
            })
    } else {
        Err(Refusal::Failed)
    };
    if outcome.is_ok() {
        desk.pause(restore_delay(chars));
    }
    let restored = desk.with_pasteboard(move |pasteboard| {
        // The whole wrapper moves to the main thread, not only the snapshot inside it.
        let kept = kept;
        pasteboard::put_back(pasteboard, kept.0, written)
    });
    outcome.map(|()| restored)
}

struct Job {
    id: u64,
    pid: i32,
    text: Text,
    /// When its request was read, on the desk's clock.
    received: Instant,
}

fn insert(desk: &impl Desk, job: Job) -> String {
    let Job {
        id,
        pid,
        text,
        received,
    } = job;
    if CLOSING.load(Ordering::Acquire) {
        return refused_line(id, Refusal::Failed);
    }
    // Begun now: after the insert before it, its restore included, for one that waited (F13).
    let times = Times {
        received,
        started: (desk.clock())(),
    };
    let outcome = autoreleasepool(|_| paste(desk, pid, &text, times));
    // Zeroed here, before the answer is written.
    drop(text);
    match outcome {
        Ok(restored) => inserted_line(id, restored),
        Err(refusal) => refused_line(id, refusal),
    }
}

/// The worker: one insert at a time, in the order the app asked, each answered once it is
/// finished, its restore included, and before the next one begins.
fn run_worker(desk: &impl Desk, queue: mpsc::Receiver<Job>, mut answer: impl FnMut(&str)) {
    for job in queue {
        answer(&insert(desk, job));
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
        run_worker(&MacDesk, queue, emit);
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
    use objc2::msg_send;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{
        NSPasteboardItem, NSPasteboardTypeRTF, NSPasteboardTypeString, NSPasteboardWriting,
    };
    use objc2_core_graphics::{CGEventField, CGEventType};
    use objc2_foundation::{NSArray, NSData};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    /// `kCGEventFlagMaskControl`.
    const CONTROL: u64 = 0x0004_0000;

    /// The worker's system for a test: a private pasteboard of its own (released when the test
    /// ends, so no test touches the general one), a post that records the text the pasteboard
    /// held when it was posted, pauses that are only recorded, and the modifiers it is told. Its
    /// clock moves only by those pauses and by the time it is told the pasteboard work takes.
    struct TestDesk {
        pasteboard: Retained<NSPasteboard>,
        /// Answers to `modifier_flags`, one per read; nothing held once they run out.
        flags: RefCell<VecDeque<u64>>,
        reads: Cell<usize>,
        /// The text on the pasteboard at each post, and how many flag reads came before it.
        posts: RefCell<Vec<(Option<String>, usize)>>,
        pauses: RefCell<Vec<Duration>>,
        checks: Cell<usize>,
        /// The check (counted from 1) that finds another app in front.
        refuse_check: Option<usize>,
        /// Whether a post fails, as `CGEventPostToPid` can when no event could be made.
        fail_post: bool,
        /// How long each pasteboard visit takes on the clock, in order; nothing once they run out.
        pasteboard_costs: RefCell<VecDeque<Duration>>,
        /// The clock: nanoseconds past `now`'s start.
        ticks: Arc<AtomicU64>,
        now: Box<dyn Fn() -> Instant + Sync>,
    }

    impl TestDesk {
        fn new() -> Self {
            let ticks = Arc::new(AtomicU64::new(0));
            let start = Instant::now();
            let read = Arc::clone(&ticks);
            Self {
                pasteboard: NSPasteboard::pasteboardWithUniqueName(),
                flags: RefCell::new(VecDeque::new()),
                reads: Cell::new(0),
                posts: RefCell::new(Vec::new()),
                pauses: RefCell::new(Vec::new()),
                checks: Cell::new(0),
                refuse_check: None,
                fail_post: false,
                pasteboard_costs: RefCell::new(VecDeque::new()),
                ticks,
                now: Box::new(move || start + Duration::from_nanos(read.load(Ordering::SeqCst))),
            }
        }

        fn advance(&self, by: Duration) {
            let nanos = u64::try_from(by.as_nanos()).expect("a test's time fits");
            self.ticks.fetch_add(nanos, Ordering::SeqCst);
        }

        /// One item per slice, each with its types and their bytes.
        fn put_items(&self, items: &[&[(&NSString, &[u8])]]) {
            self.pasteboard.clearContents();
            let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
                .iter()
                .map(|types| {
                    let item = NSPasteboardItem::new();
                    for (kind, bytes) in *types {
                        assert!(item.setData_forType(&NSData::with_bytes(bytes), kind));
                    }
                    ProtocolObject::from_retained(item)
                })
                .collect();
            assert!(self
                .pasteboard
                .writeObjects(&NSArray::from_retained_slice(&items)));
        }

        fn text(&self) -> Option<String> {
            // SAFETY: an AppKit constant.
            self.pasteboard
                .stringForType(unsafe { NSPasteboardTypeString })
                .map(|text| text.to_string())
        }

        /// Every item, every type and its bytes.
        fn contents(&self) -> Vec<Vec<(String, Vec<u8>)>> {
            self.pasteboard
                .pasteboardItems()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            item.types()
                                .iter()
                                .map(|kind| {
                                    let data = item.dataForType(&kind).map(|data| data.to_vec());
                                    (kind.to_string(), data.unwrap_or_default())
                                })
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default()
        }

        fn put(&self, types: &[(&NSString, &[u8])]) {
            self.pasteboard.clearContents();
            let item = NSPasteboardItem::new();
            for (kind, bytes) in types {
                assert!(item.setData_forType(&NSData::with_bytes(bytes), kind));
            }
            let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
                vec![ProtocolObject::from_retained(item)];
            assert!(self
                .pasteboard
                .writeObjects(&NSArray::from_retained_slice(&items)));
        }
    }

    impl Drop for TestDesk {
        fn drop(&mut self) {
            // SAFETY: `releaseGlobally` takes no arguments and returns nothing; the pasteboard is
            // not used after it.
            let () = unsafe { msg_send![&*self.pasteboard, releaseGlobally] };
        }
    }

    impl Desk for TestDesk {
        fn check(&self, _pid: i32) -> Result<(), Refusal> {
            self.checks.set(self.checks.get() + 1);
            if Some(self.checks.get()) == self.refuse_check {
                Err(Refusal::TargetChanged)
            } else {
                Ok(())
            }
        }

        fn v_key_code(&self) -> u16 {
            ANSI_V_KEY_CODE
        }

        fn with_pasteboard<T: Send>(&self, work: impl FnOnce(&NSPasteboard) -> T + Send) -> T {
            if let Some(cost) = self.pasteboard_costs.borrow_mut().pop_front() {
                self.advance(cost);
            }
            work(&self.pasteboard)
        }

        fn modifier_flags(&self) -> u64 {
            self.reads.set(self.reads.get() + 1);
            self.flags.borrow_mut().pop_front().unwrap_or(0)
        }

        fn post(&self, _pid: i32, _key_code: u16) -> bool {
            self.posts
                .borrow_mut()
                .push((self.text(), self.reads.get()));
            !self.fail_post
        }

        fn pause(&self, wait: Duration) {
            self.pauses.borrow_mut().push(wait);
            self.advance(wait);
        }

        fn clock(&self) -> &(dyn Fn() -> Instant + Sync) {
            &*self.now
        }
    }

    /// A request read now on `desk`'s clock.
    fn job(desk: &TestDesk, id: u64, text: &str) -> Job {
        job_at(id, text, (desk.now)())
    }

    fn job_at(id: u64, text: &str, received: Instant) -> Job {
        Job {
            id,
            pid: 501,
            text: Text::new(text.to_string()),
            received,
        }
    }

    fn worker_answers(desk: &TestDesk, jobs: Vec<Job>) -> Vec<serde_json::Value> {
        let (send, queue) = mpsc::channel();
        for job in jobs {
            send.send(job).unwrap();
        }
        drop(send);
        let mut answers = Vec::new();
        run_worker(desk, queue, |line| {
            answers.push(serde_json::from_str(line).expect("an answer is JSON"))
        });
        answers
    }

    /// F13 (W-A4): two dictations in a row into another app. The worker finishes the first,
    /// its restore included, before it takes the second, so the second snapshot is of what the
    /// person copied, not of the first text, and the clipboard ends as it began. The first
    /// paste waits for a Control still held after Control plus fn (F19); each restore waits as
    /// long as its own text asks (F8).
    #[test]
    fn two_inserts_in_a_row_snapshot_the_original_not_the_first_text() {
        autoreleasepool(|_| {
            let desk = TestDesk::new();
            // SAFETY: AppKit constants.
            let (string, rtf) = unsafe { (NSPasteboardTypeString, NSPasteboardTypeRTF) };
            desk.put(&[
                (string, b"what the person copied".as_slice()),
                (rtf, br"{\rtf1\ansi what the {\b person} copied}".as_slice()),
            ]);
            let before = desk.contents();
            desk.flags.borrow_mut().extend([CONTROL, CONTROL]);
            let long = "word ".repeat(4_000);

            let answers = worker_answers(
                &desk,
                vec![job(&desk, 1, "first dictation"), job(&desk, 2, &long)],
            );

            assert_eq!(
                answers,
                vec![
                    serde_json::json!({"id":1,"type":"inserted","restored":"all"}),
                    serde_json::json!({"id":2,"type":"inserted","restored":"all"}),
                ]
            );
            let posts = desk.posts.borrow().clone();
            assert_eq!(
                posts,
                vec![
                    (Some("first dictation".to_string()), 3),
                    (Some(long.clone()), 4),
                ],
                "each paste saw its own text, the first only once Control was let go"
            );
            assert_eq!(
                *desk.pauses.borrow(),
                vec![
                    crate::protocol::MODIFIER_POLL,
                    crate::protocol::MODIFIER_POLL,
                    restore_delay("first dictation".chars().count()),
                    restore_delay(long.chars().count()),
                ]
            );
            assert_eq!(
                restore_delay(long.chars().count()),
                Duration::from_millis(600)
            );
            assert_eq!(desk.contents(), before, "the clipboard ends as it began");
        });
    }

    /// An insert refused at the last look still puts the clipboard back, posts nothing, and
    /// the next insert goes ahead.
    #[test]
    fn a_refused_insert_still_puts_the_clipboard_back() {
        autoreleasepool(|_| {
            let mut desk = TestDesk::new();
            // The first insert's second check: another app came to the front.
            desk.refuse_check = Some(2);
            // SAFETY: an AppKit constant.
            let string = unsafe { NSPasteboardTypeString };
            desk.put(&[(string, b"what the person copied".as_slice())]);
            let before = desk.contents();
            let answers =
                worker_answers(&desk, vec![job(&desk, 7, "first"), job(&desk, 8, "second")]);
            assert_eq!(
                answers,
                vec![
                    serde_json::json!({"id":7,"type":"refused","reason":"target-changed"}),
                    serde_json::json!({"id":8,"type":"inserted","restored":"all"}),
                ]
            );
            assert_eq!(desk.posts.borrow().len(), 1);
            assert_eq!(desk.contents(), before);
        });
    }

    /// A Command-V that could not be posted (no event could be made): the answer is `failed`,
    /// so the app copies the text; the pasteboard is put back at once, every item and type of
    /// it, with no restore delay, because nothing was pasted that could still read the text;
    /// the dictated text is not left on it; and the next insert goes ahead.
    #[test]
    fn a_failed_post_answers_failed_and_puts_everything_back_at_once() {
        autoreleasepool(|_| {
            let mut desk = TestDesk::new();
            desk.fail_post = true;
            // SAFETY: AppKit constants.
            let (string, rtf) = unsafe { (NSPasteboardTypeString, NSPasteboardTypeRTF) };
            let private = NSString::from_str("org.biorouter.test.private-kind");
            desk.put_items(&[
                &[
                    (string, b"what the person copied".as_slice()),
                    (rtf, br"{\rtf1\ansi what the {\b person} copied}".as_slice()),
                    (&*private, [0_u8, 1, 2, 255].as_slice()),
                ],
                &[(&*private, b"a second item".as_slice())],
            ]);
            let before = desk.contents();
            assert_eq!(before.len(), 2, "two items to put back");

            let answers = worker_answers(&desk, vec![job(&desk, 4, "dictated words")]);

            assert_eq!(
                answers,
                vec![serde_json::json!({"id":4,"type":"refused","reason":"failed"})]
            );
            assert_eq!(
                *desk.posts.borrow(),
                vec![(Some("dictated words".to_string()), 1)],
                "one Command-V was tried, with the text on the pasteboard"
            );
            assert!(
                desk.pauses.borrow().is_empty(),
                "no restore delay after a post that failed"
            );
            assert_eq!(desk.contents(), before, "every item and type comes back");
            assert_ne!(desk.text().as_deref(), Some("dictated words"));

            // The worker goes on: the next insert pastes.
            desk.fail_post = false;
            let answers = worker_answers(&desk, vec![job(&desk, 5, "next words")]);
            assert_eq!(
                answers,
                vec![serde_json::json!({"id":5,"type":"inserted","restored":"all"})]
            );
            assert_eq!(desk.contents(), before);
        });
    }

    /// F13: the next dictation's text arrives while the first is still pasting, and the first
    /// is slow (its snapshot waited 1.8 s for an app fulfilling what it promised, and Control
    /// was still down for 150 ms). The second begins when the first has put the clipboard back,
    /// 2.15 s after its request arrived, and is pasted: its two seconds count from then.
    #[test]
    fn an_insert_queued_behind_a_slow_one_is_still_pasted() {
        autoreleasepool(|_| {
            let desk = TestDesk::new();
            // SAFETY: AppKit constants.
            let (string, rtf) = unsafe { (NSPasteboardTypeString, NSPasteboardTypeRTF) };
            desk.put(&[
                (string, b"what the person copied".as_slice()),
                (rtf, br"{\rtf1\ansi what the {\b person} copied}".as_slice()),
            ]);
            let before = desk.contents();
            let ms = Duration::from_millis;
            desk.pasteboard_costs.borrow_mut().push_back(ms(1_800));
            desk.flags.borrow_mut().extend([CONTROL; 15]);
            let start = (desk.now)();

            let answers = worker_answers(
                &desk,
                vec![
                    job_at(1, "first dictation", start),
                    job_at(2, "second dictation", start + ms(200)),
                ],
            );

            assert_eq!(
                answers,
                vec![
                    serde_json::json!({"id":1,"type":"inserted","restored":"all"}),
                    serde_json::json!({"id":2,"type":"inserted","restored":"all"}),
                ]
            );
            assert_eq!(
                *desk.posts.borrow(),
                vec![
                    (Some("first dictation".to_string()), 16),
                    (Some("second dictation".to_string()), 17),
                ],
                "each paste saw its own text"
            );
            // The first ended at 2.35 s (1.8 s, 150 ms of Control, 400 ms before the restore),
            // the second 400 ms later.
            assert_eq!((desk.now)().saturating_duration_since(start), ms(2_750));
            assert_eq!(desk.contents(), before, "the clipboard ends as it began");
        });
    }

    /// F9 with F13: a queued insert is never pasted so late that its answer would reach the app
    /// after `INSERT_TIMEOUT_MS` from its request, when the app has copied the text already.
    /// Behind a first insert that took 3.4 s, a text that waits 1,500 ms before its restore is
    /// refused and copied, and a short one, which waits 400 ms, is still pasted.
    #[test]
    fn a_queued_insert_is_refused_when_its_answer_would_come_after_the_app_gave_up() {
        autoreleasepool(|_| {
            let desk = TestDesk::new();
            // SAFETY: an AppKit constant.
            let string = unsafe { NSPasteboardTypeString };
            desk.put(&[(string, b"what the person copied".as_slice())]);
            let before = desk.contents();
            let ms = Duration::from_millis;
            desk.pasteboard_costs.borrow_mut().push_back(ms(1_900));
            let long = "a".repeat(110_000);
            assert_eq!(
                restore_delay(long.len()),
                crate::protocol::MAX_RESTORE_DELAY
            );
            let start = (desk.now)();

            let answers = worker_answers(
                &desk,
                vec![
                    job_at(1, &long, start),
                    job_at(2, &long, start),
                    job_at(3, "short words", start),
                ],
            );

            assert_eq!(
                answers,
                vec![
                    serde_json::json!({"id":1,"type":"inserted","restored":"all"}),
                    serde_json::json!({"id":2,"type":"refused","reason":"failed"}),
                    serde_json::json!({"id":3,"type":"inserted","restored":"all"}),
                ]
            );
            let posted: Vec<Option<String>> = desk
                .posts
                .borrow()
                .iter()
                .map(|(text, _)| text.clone())
                .collect();
            assert_eq!(
                posted,
                vec![Some(long.clone()), Some("short words".to_string())]
            );
            assert_eq!(desk.contents(), before, "the clipboard ends as it began");
        });
    }

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
