//! What the desktop app and the insertion helper say to each other, and every decision the
//! helper makes from the facts `mac.rs` reads, kept here so they are tested on every system.
//!
//! The app writes one JSON request per line on the helper's standard input and reads one JSON
//! answer per line from its standard output (`ui/desktop/src/utils/textInsertHelper.ts`,
//! `TextInsertService`):
//!
//! | Request | Answer |
//! |---|---|
//! | (start) | `{"type":"ready","v":1,"postAllowed":bool}` |
//! | `{"id":n,"op":"front"}` | `{"id":n,"type":"front","pid":int\|null,"bundle":string\|null,"secureInput":bool,"postAllowed":bool}` |
//! | `{"id":n,"op":"insert","pid":int,"text":string}` | `{"id":n,"type":"inserted","restored":...}` or `{"id":n,"type":"refused","reason":...}`; with `--test-pasteboard` only, `{"id":n,"type":"inserted","restored":...,"posted":false}` |
//! | `{"id":n,"op":"access"}` | `{"id":n,"type":"access","postAllowed":bool}` |
//! | `{"id":n,"op":"request-access"}` | the same, after asking macOS for Accessibility |
//!
//! The text of an insert is the only text that ever reaches the helper. It never appears in an
//! answer or a log line, and the helper's own buffers that held it are zeroed once it has been
//! pasted ([`Text`], [`LineReader`]). Two copies are released rather than zeroed, because the
//! helper does not own their memory: the JSON parser's scratch space for a string with escapes,
//! and the AppKit string handed to the pasteboard. Unknown fields in a request are ignored, so a
//! later optional field needs no new version.

use std::io::{self, Read};
use std::time::Duration;

use serde::Deserialize;

pub const PROTOCOL_VERSION: u32 = 1;

/// The longest request line the helper reads, in bytes without its newline. A longer line is
/// refused without being parsed or kept.
pub const MAX_REQUEST_LINE: usize = 256 * 1024;

/// How much of a refused, too long line is kept to find its request id.
pub const ID_PREFIX: usize = 64;

/// `kVK_ANSI_V`: the V key on an ANSI keyboard, used when no key of the current layout types `v`.
pub const ANSI_V_KEY_CODE: u16 = 9;

/// The highest virtual key code a keyboard layout maps (`kVK_*` codes are all below 128).
pub const LAST_KEY_CODE: u16 = 127;

/// `kCGEventFlagMaskCommand`.
pub const COMMAND_FLAG: u64 = 0x0010_0000;

/// Shift, Control, Option, Command and fn (`kCGEventFlagMask*`): the modifiers a person may
/// still be holding when the text is ready. A Command-V posted while one of them is down could
/// reach the app as another shortcut, so the helper waits for them to be released.
pub const HELD_MODIFIERS: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x0010_0000 | 0x0080_0000;

/// How long the helper waits for the modifiers to be released before it pastes anyway.
pub const MODIFIER_WAIT: Duration = Duration::from_millis(500);
pub const MODIFIER_POLL: Duration = Duration::from_millis(10);

/// How long the pasted text stays on the pasteboard, at least, before what was there is put
/// back. Too short and a slow app pastes the old contents; the owner's pass measures it per app.
pub const MIN_RESTORE_DELAY: Duration = Duration::from_millis(400);

/// The longest the pasted text stays there ([`restore_delay`]), however long it is.
pub const MAX_RESTORE_DELAY: Duration = Duration::from_millis(1_500);

/// After this long from when an insert begins the helper no longer pastes: the app gives up on an
/// insert after `INSERT_TIMEOUT_MS` (`ui/desktop/src/utils/textInsertHelper.ts`) and copies the
/// text instead, and a paste after that would be a second copy of it. An insert begins when its
/// request arrives, or, queued behind another, when that one has finished ([`may_post`]).
pub const POST_DEADLINE: Duration = Duration::from_millis(2_000);

/// The app's `INSERT_TIMEOUT_MS` (`ui/desktop/src/utils/textInsertHelper.ts`, whose test pins the
/// two equal): how long after sending an insert it waits for the answer before it copies the text
/// instead. Counted from the request's arrival, so it binds an insert that waited in the queue.
pub const APP_INSERT_TIMEOUT: Duration = Duration::from_millis(5_000);

/// The most an answer needs after its restore delay: putting the pasteboard back, writing the
/// line and the app reading it.
pub const ANSWER_MARGIN: Duration = Duration::from_millis(1_000);

/// The most a snapshot of the pasteboard keeps, in bytes. Types past it are left out and the
/// restore reports `partial`.
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// The markers clipboard managers read as "do not keep this" (nspasteboard.org).
pub const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
pub const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

/// The longest bundle identifier the helper reports; a longer one is reported as unknown.
pub const MAX_BUNDLE_LEN: usize = 255;

/// The longest name `--test-pasteboard` takes.
pub const MAX_PASTEBOARD_NAME: usize = 128;

/// Overwrite `bytes` with zeros in a way the compiler keeps.
pub fn zero_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Zero a string's whole allocation, the spare capacity included, and leave it empty.
pub fn zero_string(text: &mut String) {
    // SAFETY: only zeros are written, which keeps the contents valid UTF-8, and the string is
    // cleared right after.
    let bytes = unsafe { text.as_mut_vec() };
    zero_bytes(bytes);
    for byte in bytes.spare_capacity_mut() {
        // SAFETY: a valid, aligned pointer into the vector's own allocation.
        unsafe { std::ptr::write_volatile(byte.as_mut_ptr(), 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    bytes.clear();
}

/// The text of one insert. It is zeroed when dropped, and its `Debug` never shows it.
#[derive(PartialEq, Eq)]
pub struct Text(String);

impl Text {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for Text {
    fn drop(&mut self) {
        zero_string(&mut self.0);
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Text({} bytes)", self.0.len())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Op {
    /// The app in front, whether secure input is on and whether posting is allowed.
    Front,
    /// Paste `text` into `pid`, which must still be the app in front.
    Insert { pid: i32, text: Text },
    /// Read Accessibility (event posting) without asking.
    Access,
    /// Ask macOS for Accessibility, then read it. Sent only from a person's click in Settings.
    RequestAccess,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub op: Op,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Request(Request),
    /// Not a request the helper can serve. Answered with `refused` (`failed`) when its id could
    /// be read, ignored otherwise.
    Unusable {
        id: Option<u64>,
    },
}

#[derive(Deserialize)]
struct WireRequest {
    id: u64,
    op: String,
    #[serde(default)]
    pid: Option<i64>,
    #[serde(default)]
    text: Option<String>,
}

/// One request line, as the app writes it.
pub fn parse_request(line: &[u8]) -> Parsed {
    let wire: WireRequest = match serde_json::from_slice(line) {
        Ok(wire) => wire,
        Err(_) => {
            return Parsed::Unusable {
                id: recover_id(line),
            }
        }
    };
    let WireRequest { id, op, pid, text } = wire;
    let text = text.map(Text::new);
    let op = match op.as_str() {
        "front" => Op::Front,
        "access" => Op::Access,
        "request-access" => Op::RequestAccess,
        "insert" => {
            let pid = pid
                .and_then(|pid| i32::try_from(pid).ok())
                .filter(|pid| *pid > 0);
            match (pid, text) {
                (Some(pid), Some(text)) if !text.as_str().is_empty() => Op::Insert { pid, text },
                _ => return Parsed::Unusable { id: Some(id) },
            }
        }
        _ => return Parsed::Unusable { id: Some(id) },
    };
    Parsed::Request(Request { id, op })
}

/// The request id at the start of a line that cannot be parsed whole: the app writes `id`
/// first, so a refused line can still be answered.
pub fn recover_id(line: &[u8]) -> Option<u64> {
    let head = &line[..line.len().min(ID_PREFIX)];
    let key = b"\"id\"";
    let at = head.windows(key.len()).position(|window| window == key)?;
    let mut rest = head[at + key.len()..].iter().copied().peekable();
    while rest.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
        rest.next();
    }
    if rest.next()? != b':' {
        return None;
    }
    while rest.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
        rest.next();
    }
    let mut id: u64 = 0;
    let mut digits = 0;
    while let Some(byte) = rest.peek().copied().filter(u8::is_ascii_digit) {
        id = id.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
        digits += 1;
        rest.next();
    }
    // The number must have ended inside the kept prefix.
    (digits > 0 && rest.peek().is_some()).then_some(id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Accessibility does not allow this process to post events.
    NoPermission,
    /// Secure input is on (a password field has focus), so nothing may be typed.
    SecureInput,
    /// The app in front is no longer the one the dictation started in.
    TargetChanged,
    /// Anything else: an unusable request, a pasteboard that would not take the text, an
    /// event that could not be made, or a paste that would come too late.
    Failed,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoPermission => "no-permission",
            Self::SecureInput => "secure-input",
            Self::TargetChanged => "target-changed",
            Self::Failed => "failed",
        }
    }
}

/// What the helper read about the app in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontFacts {
    pub pid: Option<i32>,
    pub bundle: Option<String>,
    pub secure_input: bool,
    pub post_allowed: bool,
}

/// The facts an insert is judged by, read just before the pasteboard is touched and again just
/// before the paste is posted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertFacts {
    pub post_allowed: bool,
    pub secure_input: bool,
    pub front_pid: Option<i32>,
}

/// Why an insert into `target` must not happen, in the order the reasons are checked.
pub fn insert_refusal(target: i32, facts: &InsertFacts) -> Option<Refusal> {
    if !facts.post_allowed {
        Some(Refusal::NoPermission)
    } else if facts.secure_input {
        Some(Refusal::SecureInput)
    } else if facts.front_pid != Some(target) {
        Some(Refusal::TargetChanged)
    } else {
        None
    }
}

/// A bundle identifier as the helper reports it: none when it is empty or implausibly long.
pub fn reported_bundle(bundle: Option<String>) -> Option<String> {
    bundle.filter(|bundle| !bundle.is_empty() && bundle.len() <= MAX_BUNDLE_LEN)
}

/// A pid as the helper reports it: none for an app without one (`-1`).
pub fn reported_pid(pid: i32) -> Option<i32> {
    (pid > 0).then_some(pid)
}

/// The flags both Command-V events carry: Command and nothing else, so a modifier the person
/// still holds, or one the source remembers, never turns the paste into another shortcut.
pub fn paste_flags() -> u64 {
    COMMAND_FLAG
}

/// Whether a person is still holding a modifier, from the HID system's flags.
pub fn modifiers_held(flags: u64) -> bool {
    flags & HELD_MODIFIERS != 0
}

/// How long the pasted text of `chars` characters stays on the pasteboard before what was there
/// is put back: [`MIN_RESTORE_DELAY`] and 100 ms more for every 10,000 characters, at most
/// [`MAX_RESTORE_DELAY`], so a slow app given a long text still pastes it and not the old
/// contents.
pub fn restore_delay(chars: usize) -> Duration {
    let extra = u64::try_from(chars / 100).unwrap_or(u64::MAX);
    MIN_RESTORE_DELAY
        .saturating_add(Duration::from_millis(extra))
        .min(MAX_RESTORE_DELAY)
}

/// Wait, briefly, for the person to let go of the modifiers: read `held_flags` (the HID
/// system's flags) every [`MODIFIER_POLL`] until none of [`HELD_MODIFIERS`] is down, or until
/// `elapsed` says [`MODIFIER_WAIT`] has passed. `true` when they were let go in time.
pub fn wait_for_release(
    mut held_flags: impl FnMut() -> u64,
    mut elapsed: impl FnMut() -> Duration,
    mut pause: impl FnMut(Duration),
) -> bool {
    loop {
        if !modifiers_held(held_flags()) {
            return true;
        }
        if elapsed() >= MODIFIER_WAIT {
            return false;
        }
        pause(MODIFIER_POLL);
    }
}

/// What putting the pasteboard back did (`pasteboard.rs`, `restore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The pasteboard holds what it held before the text was written.
    Restored,
    /// Something was copied after the text; it is left alone.
    Moved,
    /// The pasteboard would not take the kept items back.
    Failed,
    /// What was there could not be read, so the text is left where a copy would leave it.
    NotRead,
    /// What was there was a password (concealed), so the pasteboard was cleared instead.
    Cleared,
}

/// How the person's clipboard was left after an insert, as the `inserted` answer says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    /// Everything that was there is back.
    All,
    /// Something was copied while the text was pasted, and it is left alone.
    Moved,
    /// Some of what was there could not be put back.
    Partial,
    /// What was there could not be read, so the text is still there.
    KeptText,
    /// What was there was a password, so the pasteboard was cleared rather than refilled.
    Cleared,
}

impl Restored {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Moved => "moved",
            Self::Partial => "partial",
            Self::KeptText => "kept-text",
            Self::Cleared => "cleared",
        }
    }
}

/// How the clipboard was left, from what the restore did and whether the snapshot left any type
/// out (`incomplete`). A pasteboard that would not take the kept items back was cleared first,
/// so neither the text nor all of the original is there: `partial`, not `kept-text`.
pub fn restored(outcome: RestoreOutcome, incomplete: bool) -> Restored {
    match outcome {
        RestoreOutcome::Restored if incomplete => Restored::Partial,
        RestoreOutcome::Restored => Restored::All,
        RestoreOutcome::Moved => Restored::Moved,
        RestoreOutcome::Failed => Restored::Partial,
        RestoreOutcome::NotRead => Restored::KeptText,
        RestoreOutcome::Cleared => Restored::Cleared,
    }
}

/// Whether what was on the pasteboard may be put back: only while the pasteboard still holds
/// what the helper wrote. A person's (or another app's) copy in the meantime is left alone.
pub fn should_restore(count_written: isize, count_now: isize) -> bool {
    count_written == count_now
}

/// `NSPasteboardAccessBehaviorAlwaysDeny` (macOS 15.4 and later).
pub const ACCESS_BEHAVIOR_ALWAYS_DENY: isize = 3;

/// Whether a snapshot can be put back. Not when reading the pasteboard is denied, nor when it
/// reported items and none of them could be read: putting back "nothing" would then clear the
/// person's copy along with the text, where leaving the text loses only what a copy would.
pub fn snapshot_restorable(read_denied: bool, items_reported: usize, items_kept: usize) -> bool {
    !read_denied && (items_reported == 0 || items_kept > 0)
}

/// Whether a paste would now come too late (see [`POST_DEADLINE`]).
pub fn past_deadline(since_request: Duration) -> bool {
    since_request > POST_DEADLINE
}

/// Whether the paste of a text of `chars` characters may still be posted, `since_request` after
/// its request arrived and `since_start` after its insert began (F9, F13): within
/// [`POST_DEADLINE`] of the start, so an insert queued behind a slow one still gets its own two
/// seconds, and early enough that the answer, after this text's [`restore_delay`] and
/// [`ANSWER_MARGIN`], reaches the app inside [`APP_INSERT_TIMEOUT`] from the request, before the
/// app copies the text itself. For an insert that did not wait the first rule alone decides.
pub fn may_post(since_request: Duration, since_start: Duration, chars: usize) -> bool {
    !past_deadline(since_start)
        && since_request
            .saturating_add(restore_delay(chars))
            .saturating_add(ANSWER_MARGIN)
            <= APP_INSERT_TIMEOUT
}

/// The key code that types `v` in the current layout. `translate(code, command)` is what the
/// key types with Command held (`command` true) or with no modifier; Command is asked first,
/// because a layout such as "Dvorak - QWERTY ⌘" types other letters under Command. With no key
/// typing `v` (a layout without Latin letters), the ANSI V key, which apps then match.
pub fn v_key_code(mut translate: impl FnMut(u16, bool) -> Option<String>) -> u16 {
    for command in [true, false] {
        for code in 0..=LAST_KEY_CODE {
            if translate(code, command).is_some_and(|typed| typed.eq_ignore_ascii_case("v")) {
                return code;
            }
        }
    }
    ANSI_V_KEY_CODE
}

/// How the helper was started.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// `--test-pasteboard <name>`: snapshot, write and restore that named pasteboard instead of
    /// the general one, and post no paste ([`Options::posts_paste`]). The app passes it only in an
    /// unpackaged build (`BIOROUTER_TEXTINSERT_TEST_PASTEBOARD`).
    pub test_pasteboard: Option<String>,
}

impl Options {
    /// Whether an insert posts its Command-V. Not with a test pasteboard: a Command-V makes the
    /// app paste the general pasteboard, whichever board holds the text, so it would type the
    /// person's own clipboard into the app. A test run checks every other step of an insert (the
    /// checks, the snapshot, the write, the modifier wait, the restore) and answers
    /// `posted: false` ([`inserted_unposted_line`]).
    pub fn posts_paste(&self) -> bool {
        self.test_pasteboard.is_none()
    }
}

/// A pasteboard name `--test-pasteboard` accepts: letters, digits, `.`, `_` and `-`. The system's
/// own pasteboards (`Apple CFPasteboard general` and the rest) all contain spaces, so none of
/// them can be named.
pub fn valid_pasteboard_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PASTEBOARD_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// The helper's arguments, without the program name. `None` for anything else.
pub fn parse_args(args: &[String]) -> Option<Options> {
    match args {
        [] => Some(Options::default()),
        [flag, name] if flag == "--test-pasteboard" && valid_pasteboard_name(name) => {
            Some(Options {
                test_pasteboard: Some(name.clone()),
            })
        }
        _ => None,
    }
}

fn line(value: serde_json::Value) -> String {
    value.to_string()
}

pub fn ready_line(post_allowed: bool) -> String {
    line(serde_json::json!({ "type": "ready", "v": PROTOCOL_VERSION, "postAllowed": post_allowed }))
}

pub fn front_line(id: u64, facts: &FrontFacts) -> String {
    line(serde_json::json!({
        "id": id,
        "type": "front",
        "pid": facts.pid,
        "bundle": facts.bundle,
        "secureInput": facts.secure_input,
        "postAllowed": facts.post_allowed,
    }))
}

/// The text was pasted; `restored` says how the person's clipboard was left. The app reads the
/// field leniently, so an older app ignores it and the protocol stays version 1.
pub fn inserted_line(id: u64, restored: Restored) -> String {
    line(serde_json::json!({ "id": id, "type": "inserted", "restored": restored.as_str() }))
}

/// A test pasteboard's insert ([`Options::posts_paste`]): every step ran but the paste, so nothing
/// reached the app, and `restored` says how the named board was left. Only a helper started with
/// `--test-pasteboard` writes it; the app reads the field leniently, so the protocol stays
/// version 1.
pub fn inserted_unposted_line(id: u64, restored: Restored) -> String {
    line(serde_json::json!({
        "id": id,
        "type": "inserted",
        "restored": restored.as_str(),
        "posted": false,
    }))
}

pub fn refused_line(id: u64, reason: Refusal) -> String {
    line(serde_json::json!({ "id": id, "type": "refused", "reason": reason.as_str() }))
}

pub fn access_line(id: u64, post_allowed: bool) -> String {
    line(serde_json::json!({ "id": id, "type": "access", "postAllowed": post_allowed }))
}

/// What [`LineReader::next`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// A whole line, in [`LineReader::line`].
    Line,
    /// A line over the limit; [`LineReader::line`] holds only its first [`ID_PREFIX`] bytes.
    TooLong,
    /// The app closed the pipe.
    End,
}

const CHUNK: usize = 16 * 1024;

/// Reads request lines without ever holding more than one line, and zeroes every byte it has
/// handed on. Its buffers are allocated once, so no copy of a line is left behind in memory that
/// was freed while it grew.
pub struct LineReader<R: Read> {
    inner: R,
    max: usize,
    chunk: Box<[u8]>,
    start: usize,
    end: usize,
    line: Vec<u8>,
    ended: bool,
}

impl<R: Read> LineReader<R> {
    pub fn new(inner: R, max: usize) -> Self {
        Self {
            inner,
            max,
            chunk: vec![0; CHUNK].into_boxed_slice(),
            start: 0,
            end: 0,
            line: Vec::with_capacity(max.max(ID_PREFIX)),
            ended: false,
        }
    }

    /// The line [`next`](Self::next) found.
    pub fn line(&self) -> &[u8] {
        &self.line
    }

    /// Zero the current line. Bytes read but not yet handed on stay for the next one.
    pub fn wipe(&mut self) {
        zero_bytes(&mut self.line);
        self.line.clear();
    }

    /// Whether every byte the reader holds outside the unread part of its chunk is zero.
    #[cfg(test)]
    fn holds_nothing_read(&self) -> bool {
        self.line.is_empty()
            && self.chunk[..self.start].iter().all(|byte| *byte == 0)
            && self.chunk[self.end..].iter().all(|byte| *byte == 0)
    }

    /// The next line. The previous one is zeroed first.
    pub fn next(&mut self) -> io::Result<Next> {
        self.wipe();
        let mut too_long = false;
        loop {
            if self.start == self.end {
                if self.ended {
                    // A partial last line is never handed on: the app ends every request with a
                    // newline.
                    self.wipe();
                    return Ok(Next::End);
                }
                zero_bytes(&mut self.chunk);
                self.start = 0;
                self.end = 0;
                match self.inner.read(&mut self.chunk) {
                    Ok(0) => {
                        self.ended = true;
                        continue;
                    }
                    Ok(read) => self.end = read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            let available = &self.chunk[self.start..self.end];
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.unwrap_or(available.len());
            let bytes = &available[..take];
            if !too_long && self.line.len() + bytes.len() > self.max {
                too_long = true;
                // Keep only what finding the request id needs.
                if self.line.len() > ID_PREFIX {
                    zero_bytes(&mut self.line[ID_PREFIX..]);
                    self.line.truncate(ID_PREFIX);
                }
            }
            if too_long {
                let keep = ID_PREFIX.saturating_sub(self.line.len()).min(bytes.len());
                self.line.extend_from_slice(&bytes[..keep]);
            } else {
                self.line.extend_from_slice(bytes);
            }
            let consumed = take + usize::from(newline.is_some());
            zero_bytes(&mut self.chunk[self.start..self.start + consumed]);
            self.start += consumed;
            if newline.is_some() {
                if too_long {
                    return Ok(Next::TooLong);
                }
                if self.line.last() == Some(&b'\r') {
                    let last = self.line.len() - 1;
                    zero_bytes(&mut self.line[last..]);
                    self.line.truncate(last);
                }
                if self.line.iter().all(u8::is_ascii_whitespace) {
                    self.wipe();
                    continue;
                }
                return Ok(Next::Line);
            }
        }
    }
}

impl<R: Read> Drop for LineReader<R> {
    fn drop(&mut self) {
        self.wipe();
        zero_bytes(&mut self.chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn insert(text: &str) -> Op {
        Op::Insert {
            pid: 501,
            text: Text::new(text.to_string()),
        }
    }

    #[test]
    fn every_request_the_app_writes_is_read() {
        assert_eq!(
            parse_request(br#"{"id":1,"op":"front"}"#),
            Parsed::Request(Request {
                id: 1,
                op: Op::Front
            })
        );
        assert_eq!(
            parse_request(br#"{"id":2,"op":"insert","pid":501,"text":"hello \"there\"\n"}"#),
            Parsed::Request(Request {
                id: 2,
                op: insert("hello \"there\"\n")
            })
        );
        assert_eq!(
            parse_request(br#"{"id":3,"op":"access"}"#),
            Parsed::Request(Request {
                id: 3,
                op: Op::Access
            })
        );
        assert_eq!(
            parse_request(br#"{"id":4,"op":"request-access"}"#),
            Parsed::Request(Request {
                id: 4,
                op: Op::RequestAccess
            })
        );
        // Unknown fields are ignored, so a later optional field needs no version.
        assert_eq!(
            parse_request(br#"{"id":5,"op":"front","later":true}"#),
            Parsed::Request(Request {
                id: 5,
                op: Op::Front
            })
        );
        // Text outside the Basic Multilingual Plane survives the escapes.
        assert_eq!(
            parse_request(br#"{"id":6,"op":"insert","pid":501,"text":"caf\u00e9 \ud83c\udf0d"}"#),
            Parsed::Request(Request {
                id: 6,
                op: insert("café 🌍")
            })
        );
    }

    #[test]
    fn a_request_it_cannot_serve_is_answered_by_id_when_it_has_one() {
        for line in [
            &br#"{"id":7,"op":"paste"}"#[..],
            br#"{"id":7,"op":"insert","text":"words"}"#,
            br#"{"id":7,"op":"insert","pid":0,"text":"words"}"#,
            br#"{"id":7,"op":"insert","pid":-4,"text":"words"}"#,
            br#"{"id":7,"op":"insert","pid":9999999999,"text":"words"}"#,
            br#"{"id":7,"op":"insert","pid":501,"text":""}"#,
            br#"{"id":7,"op":"insert","pid":501}"#,
            br#"{"id":7,"op":"insert","pid":501,"text":5}"#,
            br#"{"id":7, "op":"insert""#,
        ] {
            assert_eq!(
                parse_request(line),
                Parsed::Unusable { id: Some(7) },
                "{}",
                String::from_utf8_lossy(line)
            );
        }
        for line in [
            &b"not json"[..],
            br#"{"op":"front"}"#,
            br#"{"id":-1,"op":"front"}"#,
            b"",
        ] {
            assert_eq!(parse_request(line), Parsed::Unusable { id: None });
        }
    }

    #[test]
    fn the_answers_are_the_lines_the_app_parses() {
        let parsed = |line: String| serde_json::from_str::<serde_json::Value>(&line).unwrap();
        assert_eq!(
            parsed(ready_line(true)),
            serde_json::json!({"type":"ready","v":1,"postAllowed":true})
        );
        assert_eq!(
            parsed(front_line(
                9,
                &FrontFacts {
                    pid: Some(501),
                    bundle: Some("com.apple.TextEdit".into()),
                    secure_input: false,
                    post_allowed: true,
                }
            )),
            serde_json::json!({"id":9,"type":"front","pid":501,"bundle":"com.apple.TextEdit","secureInput":false,"postAllowed":true})
        );
        assert_eq!(
            parsed(front_line(
                10,
                &FrontFacts {
                    pid: None,
                    bundle: None,
                    secure_input: true,
                    post_allowed: false,
                }
            )),
            serde_json::json!({"id":10,"type":"front","pid":null,"bundle":null,"secureInput":true,"postAllowed":false})
        );
        for (restored, word) in [
            (Restored::All, "all"),
            (Restored::Moved, "moved"),
            (Restored::Partial, "partial"),
            (Restored::KeptText, "kept-text"),
            (Restored::Cleared, "cleared"),
        ] {
            assert_eq!(
                parsed(inserted_line(11, restored)),
                serde_json::json!({"id":11,"type":"inserted","restored":word})
            );
        }
        for (reason, word) in [
            (Refusal::NoPermission, "no-permission"),
            (Refusal::SecureInput, "secure-input"),
            (Refusal::TargetChanged, "target-changed"),
            (Refusal::Failed, "failed"),
        ] {
            assert_eq!(
                parsed(refused_line(12, reason)),
                serde_json::json!({"id":12,"type":"refused","reason":word})
            );
        }
        assert_eq!(
            parsed(access_line(13, false)),
            serde_json::json!({"id":13,"type":"access","postAllowed":false})
        );
        // One line each: an answer never spans two.
        for answer in [
            ready_line(false),
            inserted_line(1, Restored::Partial),
            access_line(1, true),
        ] {
            assert!(!answer.contains('\n'));
        }
    }

    #[test]
    fn a_request_and_its_answer_round_trip() {
        let mut reader = LineReader::new(
            Cursor::new(
                b"{\"id\":21,\"op\":\"front\"}\n{\"id\":22,\"op\":\"insert\",\"pid\":77,\"text\":\"two words\"}\r\n\n{\"id\":23,\"op\":\"access\"}\n".to_vec(),
            ),
            MAX_REQUEST_LINE,
        );
        let mut answers = Vec::new();
        loop {
            match reader.next().unwrap() {
                Next::End => break,
                Next::TooLong => panic!("no line here is too long"),
                Next::Line => {
                    let parsed = parse_request(reader.line());
                    reader.wipe();
                    let Parsed::Request(request) = parsed else {
                        panic!("every line here is a request")
                    };
                    answers.push(match request.op {
                        Op::Front => front_line(
                            request.id,
                            &FrontFacts {
                                pid: Some(77),
                                bundle: None,
                                secure_input: false,
                                post_allowed: true,
                            },
                        ),
                        Op::Insert { pid, text } => {
                            assert_eq!((pid, text.as_str()), (77, "two words"));
                            inserted_line(request.id, Restored::All)
                        }
                        Op::Access | Op::RequestAccess => access_line(request.id, true),
                    });
                }
            }
        }
        let answers: Vec<serde_json::Value> = answers
            .iter()
            .map(|answer| serde_json::from_str(answer).unwrap())
            .collect();
        assert_eq!(
            answers,
            vec![
                serde_json::json!({"id":21,"type":"front","pid":77,"bundle":null,"secureInput":false,"postAllowed":true}),
                serde_json::json!({"id":22,"type":"inserted","restored":"all"}),
                serde_json::json!({"id":23,"type":"access","postAllowed":true}),
            ]
        );
        assert!(reader.holds_nothing_read());
    }

    #[test]
    fn a_line_over_256_kib_is_refused_and_the_next_one_is_still_read() {
        let head = br#"{"id":31,"op":"insert","pid":501,"text":""#;
        let tail = br#""}"#;
        let fill = |total: usize| {
            let mut line = head.to_vec();
            line.resize(total - tail.len(), b'a');
            line.extend_from_slice(tail);
            line
        };
        let mut input = fill(MAX_REQUEST_LINE + 1);
        input.push(b'\n');
        input.extend_from_slice(&fill(MAX_REQUEST_LINE));
        input.push(b'\n');
        input.extend_from_slice(b"{\"id\":33,\"op\":\"front\"}\n");
        let mut reader = LineReader::new(Cursor::new(input), MAX_REQUEST_LINE);

        assert_eq!(reader.next().unwrap(), Next::TooLong);
        assert!(reader.line().len() <= ID_PREFIX);
        assert_eq!(recover_id(reader.line()), Some(31));

        assert_eq!(reader.next().unwrap(), Next::Line);
        assert_eq!(reader.line().len(), MAX_REQUEST_LINE);
        let Parsed::Request(Request {
            id: 31,
            op: Op::Insert { text, .. },
        }) = parse_request(reader.line())
        else {
            panic!("a line of exactly the limit is read")
        };
        assert_eq!(
            text.as_str().len(),
            MAX_REQUEST_LINE - head.len() - tail.len()
        );

        assert_eq!(reader.next().unwrap(), Next::Line);
        assert_eq!(
            parse_request(reader.line()),
            Parsed::Request(Request {
                id: 33,
                op: Op::Front
            })
        );
        assert_eq!(reader.next().unwrap(), Next::End);
        assert!(reader.holds_nothing_read());
    }

    #[test]
    fn a_partial_last_line_is_never_handed_on() {
        let mut reader = LineReader::new(
            Cursor::new(br#"{"id":41,"op":"insert","pid":5,"text":"half"#.to_vec()),
            MAX_REQUEST_LINE,
        );
        assert_eq!(reader.next().unwrap(), Next::End);
        assert!(reader.holds_nothing_read());
    }

    #[test]
    fn the_id_is_found_only_at_the_start_of_a_refused_line() {
        assert_eq!(recover_id(br#"{"id":12345,"op":"insert""#), Some(12345));
        assert_eq!(recover_id(br#"{ "id" : 8 ,"op""#), Some(8));
        // Cut inside the number: the number cannot be trusted.
        assert_eq!(recover_id(br#"{"id":123"#), None);
        assert_eq!(recover_id(br#"{"op":"insert","text":"id"}"#), None);
        let mut late = vec![b' '; ID_PREFIX];
        late.extend_from_slice(br#""id":3,"#);
        assert_eq!(recover_id(&late), None);
    }

    #[test]
    fn each_refusal_comes_from_its_own_fact() {
        let allowed = InsertFacts {
            post_allowed: true,
            secure_input: false,
            front_pid: Some(501),
        };
        assert_eq!(insert_refusal(501, &allowed), None);
        assert_eq!(
            insert_refusal(
                501,
                &InsertFacts {
                    post_allowed: false,
                    ..allowed
                }
            ),
            Some(Refusal::NoPermission)
        );
        assert_eq!(
            insert_refusal(
                501,
                &InsertFacts {
                    secure_input: true,
                    ..allowed
                }
            ),
            Some(Refusal::SecureInput)
        );
        assert_eq!(
            insert_refusal(
                501,
                &InsertFacts {
                    front_pid: Some(502),
                    ..allowed
                }
            ),
            Some(Refusal::TargetChanged)
        );
        assert_eq!(
            insert_refusal(
                501,
                &InsertFacts {
                    front_pid: None,
                    ..allowed
                }
            ),
            Some(Refusal::TargetChanged)
        );
        // Permission is judged first, then secure input, then the target.
        assert_eq!(
            insert_refusal(
                501,
                &InsertFacts {
                    post_allowed: false,
                    secure_input: true,
                    front_pid: None,
                }
            ),
            Some(Refusal::NoPermission)
        );
        assert_eq!(
            insert_refusal(
                501,
                &InsertFacts {
                    post_allowed: true,
                    secure_input: true,
                    front_pid: None,
                }
            ),
            Some(Refusal::SecureInput)
        );
    }

    #[test]
    fn the_paste_carries_command_and_nothing_else() {
        assert_eq!(paste_flags(), 0x0010_0000);
        for other in [
            0x0001_0000u64, // Caps Lock
            0x0002_0000,    // Shift
            0x0004_0000,    // Control
            0x0008_0000,    // Option
            0x0020_0000,    // numeric pad
            0x0040_0000,    // Help
            0x0080_0000,    // fn
            0x0000_0100,    // non-coalesced
        ] {
            assert_eq!(paste_flags() & other, 0);
        }
    }

    #[test]
    fn a_held_modifier_is_waited_for_and_caps_lock_is_not() {
        assert!(!modifiers_held(0));
        assert!(!modifiers_held(0x0001_0000)); // Caps Lock stays on for minutes
        assert!(!modifiers_held(0x0000_0100));
        for held in [
            0x0002_0000u64,
            0x0004_0000,
            0x0008_0000,
            0x0010_0000,
            0x0080_0000,
        ] {
            assert!(modifiers_held(held));
            assert!(modifiers_held(held | 0x0001_0000));
        }
    }

    #[test]
    fn the_layout_key_is_used_and_ansi_v_is_the_fallback() {
        // QWERTY: V is key 9 with or without Command.
        let qwerty = |code: u16, _command: bool| (code == 9).then(|| "v".to_string());
        assert_eq!(v_key_code(qwerty), 9);
        // Dvorak: `v` is on the key ANSI calls period (47).
        let dvorak = |code: u16, _command: bool| match code {
            47 => Some("v".to_string()),
            9 => Some(".".to_string()),
            _ => Some("x".to_string()),
        };
        assert_eq!(v_key_code(dvorak), 47);
        // "Dvorak - QWERTY ⌘": Command switches to QWERTY, and Command is what the paste holds.
        let dvorak_qwerty_command = |code: u16, command: bool| match (code, command) {
            (9, true) | (47, false) => Some("v".to_string()),
            _ => None,
        };
        assert_eq!(v_key_code(dvorak_qwerty_command), 9);
        // Upper case counts (a layout with Caps Lock in its table).
        assert_eq!(
            v_key_code(|code, _| (code == 30).then(|| "V".to_string())),
            30
        );
        // No Latin letters (Russian, Greek), or no layout data at all: the ANSI V key.
        let cyrillic = |code: u16, _command: bool| (code == 9).then(|| "м".to_string());
        assert_eq!(v_key_code(cyrillic), ANSI_V_KEY_CODE);
        assert_eq!(v_key_code(|_, _| None), ANSI_V_KEY_CODE);
    }

    /// Rule change (DD-102, F8 and F9): the restore waits longer for a long text, up to
    /// [`MAX_RESTORE_DELAY`], so the app's timeout rose from three seconds to five
    /// (`INSERT_TIMEOUT_MS` in `ui/desktop/src/utils/textInsertHelper.ts`). This test pinned
    /// three seconds against a fixed 400 ms restore; it now pins the longest insert inside five.
    #[test]
    fn the_restore_waits_for_its_own_write_and_the_paste_for_its_deadline() {
        assert!(should_restore(41, 41));
        assert!(!should_restore(41, 42));
        assert!(!past_deadline(Duration::from_millis(1_999)));
        assert!(!past_deadline(POST_DEADLINE));
        assert!(past_deadline(Duration::from_millis(2_001)));
        // The whole insert answers inside the app's five seconds, the longest restore included.
        assert!(
            POST_DEADLINE + MAX_RESTORE_DELAY + Duration::from_millis(500) < Duration::from_secs(5)
        );
        assert_eq!(POST_DEADLINE, Duration::from_millis(2_000));
        assert_eq!(MAX_RESTORE_DELAY, Duration::from_millis(1_500));
    }

    /// F13 and F9: an insert queued behind a slow one has [`POST_DEADLINE`] from when it begins,
    /// not from when its request arrived, so a quick second dictation is still pasted; and it is
    /// never posted so late that its answer, after its own restore delay, would reach the app
    /// after [`APP_INSERT_TIMEOUT`], when the app has copied the text already.
    #[test]
    fn a_queued_paste_has_its_own_deadline_and_still_answers_before_the_app_gives_up() {
        let ms = Duration::from_millis;
        // An insert that did not wait: the post deadline alone decides, for any length of text,
        // because its answer always comes inside the app's timeout.
        assert!(POST_DEADLINE + MAX_RESTORE_DELAY + ANSWER_MARGIN <= APP_INSERT_TIMEOUT);
        for chars in [0, 13, 110_000, usize::MAX] {
            for since in [0, 1_999, 2_000, 2_001, 4_000] {
                assert_eq!(
                    may_post(ms(since), ms(since), chars),
                    !past_deadline(ms(since)),
                    "{chars} characters, {since} ms"
                );
            }
        }
        // Queued behind an insert that took 2.15 s: begun just now, so it is pasted.
        assert!(may_post(ms(2_150), ms(0), 13));
        assert!(may_post(ms(2_150), ms(2_000), 13));
        assert!(!may_post(ms(2_150), ms(2_001), 13));
        // The app's five seconds, counted from the request: 400 ms of restore and the margin.
        assert!(may_post(ms(3_600), ms(0), 13));
        assert!(!may_post(ms(3_601), ms(0), 13));
        // A long text waits 1,500 ms before the restore, so it must be posted sooner.
        assert!(may_post(ms(2_500), ms(0), 110_000));
        assert!(!may_post(ms(2_501), ms(0), 110_000));
        assert!(!may_post(Duration::MAX, ms(0), 0));
    }

    /// F8: 400 ms, and 100 ms more for every 10,000 characters, at most 1,500 ms, so a slow app
    /// pastes the new text and not the old.
    #[test]
    fn the_restore_delay_grows_with_the_text_and_stops_at_its_cap() {
        let ms = |chars: usize| restore_delay(chars).as_millis();
        assert_eq!(ms(0), 400);
        assert_eq!(ms(1), 400);
        assert_eq!(ms(99), 400);
        assert_eq!(ms(100), 401);
        assert_eq!(ms(10_000), 500);
        assert_eq!(ms(25_000), 650);
        assert_eq!(ms(60_000), 1_000);
        assert_eq!(ms(110_000), 1_500);
        assert_eq!(ms(110_001), 1_500);
        assert_eq!(ms(MAX_REQUEST_LINE), 1_500);
        assert_eq!(ms(usize::MAX), 1_500);
        // It never shrinks as the text grows.
        let mut last = Duration::ZERO;
        for chars in (0..200_000).step_by(997) {
            let delay = restore_delay(chars);
            assert!(delay >= last && delay >= MIN_RESTORE_DELAY && delay <= MAX_RESTORE_DELAY);
            last = delay;
        }
    }

    /// How the clipboard was left, from what the restore did and whether the snapshot kept
    /// everything: only a restore that put everything back is `all`.
    #[test]
    fn the_inserted_answer_reports_how_the_clipboard_was_left() {
        assert_eq!(restored(RestoreOutcome::Restored, false), Restored::All);
        assert_eq!(restored(RestoreOutcome::Restored, true), Restored::Partial);
        assert_eq!(restored(RestoreOutcome::Moved, false), Restored::Moved);
        assert_eq!(restored(RestoreOutcome::Moved, true), Restored::Moved);
        assert_eq!(restored(RestoreOutcome::NotRead, false), Restored::KeptText);
        assert_eq!(restored(RestoreOutcome::NotRead, true), Restored::KeptText);
        assert_eq!(restored(RestoreOutcome::Cleared, false), Restored::Cleared);
        assert_eq!(restored(RestoreOutcome::Cleared, true), Restored::Cleared);
        // The pasteboard was cleared before it refused the kept items, so the text is gone
        // too: not everything came back.
        assert_eq!(restored(RestoreOutcome::Failed, false), Restored::Partial);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&inserted_line(
                3,
                restored(RestoreOutcome::Cleared, false)
            ))
            .unwrap(),
            serde_json::json!({"id":3,"type":"inserted","restored":"cleared"})
        );
    }

    /// F19 (W-A5): Control, Option, Shift and fn each still held when the text is ready are
    /// waited for, poll by poll, and the wait ends the moment they are let go; one held past
    /// the wait is given up on at [`MODIFIER_WAIT`]. Caps Lock is never waited for.
    #[test]
    fn control_option_shift_and_fn_held_are_each_waited_for() {
        for (name, held) in [
            ("Control", 0x0004_0000u64),
            ("Option", 0x0008_0000),
            ("Shift", 0x0002_0000),
            ("fn", 0x0080_0000),
            ("Command", 0x0010_0000),
        ] {
            // Held for three reads, then let go (Caps Lock stays on throughout).
            let reads = std::cell::Cell::new(0);
            let pauses = std::cell::RefCell::new(Vec::new());
            let released = wait_for_release(
                || {
                    reads.set(reads.get() + 1);
                    if reads.get() <= 3 {
                        held | 0x0001_0000
                    } else {
                        0x0001_0000
                    }
                },
                || MODIFIER_POLL * pauses.borrow().len() as u32,
                |wait| pauses.borrow_mut().push(wait),
            );
            assert!(released, "{name}");
            assert_eq!(reads.get(), 4, "{name}");
            assert_eq!(*pauses.borrow(), vec![MODIFIER_POLL; 3], "{name}");

            // Held throughout: given up on at the limit, never later.
            let clock = std::cell::Cell::new(Duration::ZERO);
            let paused = std::cell::Cell::new(0);
            let released = wait_for_release(
                || held,
                || clock.get(),
                |wait| {
                    clock.set(clock.get() + wait);
                    paused.set(paused.get() + 1);
                },
            );
            assert!(!released, "{name}");
            assert_eq!(clock.get(), MODIFIER_WAIT, "{name}");
            assert_eq!(
                paused.get(),
                (MODIFIER_WAIT.as_millis() / MODIFIER_POLL.as_millis()) as usize
            );
        }
        // Nothing held, or Caps Lock alone: no wait at all.
        for flags in [0u64, 0x0001_0000] {
            let paused = std::cell::Cell::new(false);
            assert!(wait_for_release(
                || flags,
                || Duration::ZERO,
                |_| paused.set(true)
            ));
            assert!(!paused.get());
        }
    }

    #[test]
    fn a_pasteboard_that_could_not_be_read_is_never_cleared() {
        assert!(snapshot_restorable(false, 0, 0));
        assert!(snapshot_restorable(false, 3, 3));
        assert!(snapshot_restorable(false, 3, 1));
        assert!(!snapshot_restorable(false, 2, 0));
        assert!(!snapshot_restorable(true, 0, 0));
        assert!(!snapshot_restorable(true, 3, 3));
    }

    #[test]
    fn only_a_plain_named_pasteboard_can_stand_in_for_the_general_one() {
        assert_eq!(parse_args(&[]), Some(Options::default()));
        assert_eq!(
            parse_args(&[
                "--test-pasteboard".into(),
                "biorouter.insert-probe_1".into()
            ]),
            Some(Options {
                test_pasteboard: Some("biorouter.insert-probe_1".into())
            })
        );
        for bad in [
            vec!["--test-pasteboard".to_string()],
            vec![
                "--test-pasteboard".into(),
                "Apple CFPasteboard general".into(),
            ],
            vec!["--test-pasteboard".into(), String::new()],
            vec!["--test-pasteboard".into(), "a/b".into()],
            vec![
                "--test-pasteboard".into(),
                "x".repeat(MAX_PASTEBOARD_NAME + 1),
            ],
            vec!["--other".into()],
            vec!["--test-pasteboard".into(), "a".into(), "extra".into()],
        ] {
            assert_eq!(parse_args(&bad), None, "{bad:?}");
        }
    }

    /// A test pasteboard posts no paste: a Command-V would make the app paste the general one.
    /// Only then does the answer say `posted: false`; the production answer is unchanged.
    #[test]
    fn only_a_helper_without_a_test_pasteboard_posts_the_paste() {
        assert!(Options::default().posts_paste());
        assert!(parse_args(&[]).is_some_and(|options| options.posts_paste()));
        let seam =
            parse_args(&["--test-pasteboard".into(), "br-live2-pb".into()]).expect("a plain name");
        assert!(!seam.posts_paste());

        let parsed = |line: String| serde_json::from_str::<serde_json::Value>(&line).unwrap();
        assert_eq!(
            parsed(inserted_unposted_line(17, Restored::All)),
            serde_json::json!({"id":17,"type":"inserted","restored":"all","posted":false})
        );
        assert_eq!(
            parsed(inserted_unposted_line(18, Restored::Cleared)),
            serde_json::json!({"id":18,"type":"inserted","restored":"cleared","posted":false})
        );
        assert!(!inserted_unposted_line(1, Restored::Partial).contains('\n'));
        // The production answer carries no `posted` at all.
        assert!(!inserted_line(19, Restored::All).contains("posted"));
    }

    #[test]
    fn reported_facts_drop_what_is_not_an_app() {
        assert_eq!(reported_pid(-1), None);
        assert_eq!(reported_pid(0), None);
        assert_eq!(reported_pid(501), Some(501));
        assert_eq!(reported_bundle(Some(String::new())), None);
        assert_eq!(reported_bundle(Some("x".repeat(MAX_BUNDLE_LEN + 1))), None);
        assert_eq!(
            reported_bundle(Some("com.apple.Notes".into())),
            Some("com.apple.Notes".into())
        );
    }

    #[test]
    fn text_is_zeroed_and_never_printed() {
        let text = Text::new("a private sentence".to_string());
        assert_eq!(format!("{text:?}"), "Text(18 bytes)");
        let mut kept = String::with_capacity(64);
        kept.push_str("a private sentence");
        zero_string(&mut kept);
        assert!(kept.is_empty());
        // SAFETY: reading the allocation `zero_string` just zeroed, within its capacity.
        let all = unsafe { std::slice::from_raw_parts(kept.as_ptr(), kept.capacity()) };
        assert!(all.iter().all(|byte| *byte == 0));
    }
}
