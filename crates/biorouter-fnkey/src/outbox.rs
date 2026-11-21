//! The line queue between the event tap and standard output.
//!
//! macOS runs the tap's callback on the run loop and turns the tap off
//! (`kCGEventTapDisabledByTimeout`) when the callback is slow. A write to standard output blocks
//! while the app is not reading the pipe, so the callback never writes: it queues the line in a
//! bounded [`Outbox`] and returns, and one writer thread ([`SharedOutbox::write_lines`]) does the
//! writing. The lines themselves are unchanged (`protocol.rs`).
//!
//! When the app has stopped reading long enough to fill the queue, a line has to go. Which one is
//! decided by what the app does with it (`ui/desktop/src/utils/fnGesture.ts`), so that what it
//! reads later still describes what fn did:
//!
//! 1. An `other` line that changes nothing: one that follows another `other` with no fn line
//!    between them. Only the first key in a press spoils it, and only the first key after a
//!    release ends the wait for a second tap; the app ignores the rest.
//! 2. A status line (`ready`, `permission`, `listener-lost`) with a newer one of its kind queued
//!    behind it: the app keeps only the latest.
//! 3. The oldest whole press: its fn down, its fn up, the keys inside it and the key that followed
//!    it, all together, so the app never sees half of a press.
//!
//! The first `other` of a press is never dropped on its own, although it is only timing: it is
//! what marks fn with an arrow or a letter as a shortcut, and without it the app would read that
//! press as dictation. An fn up whose fn down was already written is never dropped, nor the fn
//! down of a press still held, so a hold the app saw begin is always seen to end (a blind gap's
//! reset in `FnTracker::reset_after_blind_gap` reports its fn up through the same queue).
//!
//! With none of those left to drop, the queue holds at most eight lines: an fn up, an fn down,
//! one `other` in each of the three stretches around them and one line of each status kind. So
//! with any bound above eight a full queue always has a line to drop, and the queue never grows
//! past [`CAPACITY`]. (Were it ever to have none, the new line would be queued anyway: a line
//! the app needs is worth more than the bound.)

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::protocol::Signal;

/// Lines the queue holds before it drops one. A busy app reads far fewer behind.
pub const CAPACITY: usize = 256;
const _: () = assert!(CAPACITY > 8);

/// What a queued line is, for the overflow policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    FnDown,
    FnUp,
    Other,
    Ready,
    Permission,
    ListenerLost,
}

impl LineKind {
    pub fn of(signal: Signal) -> Self {
        match signal {
            Signal::FnDown => Self::FnDown,
            Signal::FnUp => Self::FnUp,
            Signal::Other => Self::Other,
        }
    }

    fn is_fn(self) -> bool {
        matches!(self, Self::FnDown | Self::FnUp)
    }

    fn is_status(self) -> bool {
        matches!(self, Self::Ready | Self::Permission | Self::ListenerLost)
    }
}

/// The bounded queue and its overflow policy, with no threads in it.
#[derive(Debug)]
pub struct Outbox<T> {
    lines: VecDeque<(LineKind, T)>,
    capacity: usize,
    /// Whether an `other` was written since the last fn line written: then the next queued
    /// `other` before an fn line changes nothing for the app.
    written_other_since_fn: bool,
    dropped: usize,
}

impl<T> Outbox<T> {
    pub const fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            capacity,
            written_other_since_fn: false,
            dropped: 0,
        }
    }

    /// Queue `line`, first dropping one line or one press by the policy if the queue is full.
    pub fn push(&mut self, kind: LineKind, line: T) {
        if self.lines.len() >= self.capacity {
            self.make_room();
        }
        self.lines.push_back((kind, line));
    }

    /// The oldest line, for the writer.
    pub fn pop(&mut self) -> Option<T> {
        let (kind, line) = self.lines.pop_front()?;
        if kind.is_fn() {
            self.written_other_since_fn = false;
        } else if kind == LineKind::Other {
            self.written_other_since_fn = true;
        }
        Some(line)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// How many lines the policy has dropped.
    #[cfg(test)]
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    fn make_room(&mut self) -> bool {
        if let Some(index) = self.first_redundant_other() {
            self.remove(&[index]);
            return true;
        }
        if let Some(index) = self.first_superseded_status() {
            self.remove(&[index]);
            return true;
        }
        if let Some(press) = self.first_whole_press() {
            self.remove(&press);
            return true;
        }
        false
    }

    /// Rule 1: an `other` after another `other`, written or queued, with no fn line between.
    fn first_redundant_other(&self) -> Option<usize> {
        let mut other_since_fn = self.written_other_since_fn;
        for (index, (kind, _)) in self.lines.iter().enumerate() {
            match kind {
                kind if kind.is_fn() => other_since_fn = false,
                LineKind::Other if other_since_fn => return Some(index),
                LineKind::Other => other_since_fn = true,
                _ => {}
            }
        }
        None
    }

    /// Rule 2: a status line with a newer one of its kind behind it.
    fn first_superseded_status(&self) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .find_map(|(index, (kind, _))| {
                (kind.is_status()
                    && self
                        .lines
                        .iter()
                        .skip(index + 1)
                        .any(|(later, _)| later == kind))
                .then_some(index)
            })
    }

    /// Rule 3: the oldest fn down whose fn up is queued too, with the `other` lines between them
    /// and the `other` right after the up (before the next fn line). Status lines in between stay.
    fn first_whole_press(&self) -> Option<Vec<usize>> {
        let down = self
            .lines
            .iter()
            .position(|(kind, _)| *kind == LineKind::FnDown)?;
        let mut press = vec![down];
        let mut released = false;
        for (index, (kind, _)) in self.lines.iter().enumerate().skip(down + 1) {
            match (released, kind) {
                (false, LineKind::Other) => press.push(index),
                (false, LineKind::FnUp) => {
                    press.push(index);
                    released = true;
                }
                (true, LineKind::Other) => {
                    press.push(index);
                    break;
                }
                (_, kind) if kind.is_fn() => break,
                _ => {}
            }
        }
        released.then_some(press)
    }

    /// Remove the lines at `indices` (ascending).
    fn remove(&mut self, indices: &[usize]) {
        for index in indices.iter().rev() {
            self.lines.remove(*index);
        }
        self.dropped += indices.len();
    }
}

/// The queue shared by the threads that make lines and the one that writes them.
pub struct SharedOutbox {
    state: Mutex<State>,
    /// A line was queued.
    queued: Condvar,
    /// The writer found the queue empty.
    drained: Condvar,
}

struct State {
    outbox: Outbox<String>,
    /// The writer holds a line it has not finished writing.
    writing: bool,
}

impl SharedOutbox {
    pub const fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(State {
                outbox: Outbox::new(capacity),
                writing: false,
            }),
            queued: Condvar::new(),
            drained: Condvar::new(),
        }
    }

    /// A poisoned lock only means some thread panicked holding it; the queue is still whole, and
    /// the tap's callback must never panic across the C boundary.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue a line. Never waits on standard output: the writer holds the lock only to take a
    /// line, never while it writes one.
    pub fn push(&self, kind: LineKind, line: String) {
        self.lock().outbox.push(kind, line);
        self.queued.notify_one();
    }

    /// The writer's loop: each line as it is queued, flushed at once so the app reads it now.
    /// Returns the error of the first write that fails, which means the app is gone.
    pub fn write_lines(&self, out: &mut impl Write) -> std::io::Error {
        loop {
            let line = {
                let mut state = self.lock();
                loop {
                    if let Some(line) = state.outbox.pop() {
                        state.writing = true;
                        break line;
                    }
                    state.writing = false;
                    self.drained.notify_all();
                    state = self
                        .queued
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            if let Err(error) = writeln!(out, "{line}").and_then(|()| out.flush()) {
                return error;
            }
        }
    }

    /// Wait up to `limit` for every queued line to be written. `false` when the time ran out.
    pub fn flush_for(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut state = self.lock();
        while state.writing || !state.outbox.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = match self.drained.wait_timeout(state, left) {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        true
    }

    #[cfg(test)]
    fn snapshot(&self) -> (usize, usize) {
        let state = self.lock();
        (state.outbox.len(), state.outbox.dropped())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{signal_line, FnTracker, SECONDARY_FN};
    use LineKind::{FnDown, FnUp, Other};

    fn drain<T>(outbox: &mut Outbox<T>) -> Vec<T> {
        std::iter::from_fn(|| outbox.pop()).collect()
    }

    fn filled(capacity: usize, lines: &[(LineKind, &'static str)]) -> Outbox<&'static str> {
        let mut outbox = Outbox::new(capacity);
        for (kind, line) in lines {
            outbox.push(*kind, *line);
        }
        outbox
    }

    #[test]
    fn below_capacity_every_line_is_kept_in_order() {
        let mut outbox = filled(
            8,
            &[(FnDown, "d"), (Other, "o1"), (Other, "o2"), (FnUp, "u")],
        );
        assert_eq!(drain(&mut outbox), vec!["d", "o1", "o2", "u"]);
        assert_eq!(outbox.dropped(), 0);
    }

    /// Rule 1: the second key in a press goes first, never the first, which spoils the press.
    #[test]
    fn a_full_queue_drops_the_oldest_key_that_changes_nothing() {
        let mut outbox = filled(
            5,
            &[
                (FnDown, "d"),
                (Other, "o1"),
                (Other, "o2"),
                (Other, "o3"),
                (FnUp, "u"),
            ],
        );
        outbox.push(LineKind::Ready, "ready");
        assert_eq!(drain(&mut outbox), vec!["d", "o1", "o3", "u", "ready"]);
        assert_eq!(outbox.dropped(), 1);
    }

    /// The writer already wrote this press's first key, so every queued key in it is redundant.
    #[test]
    fn a_key_written_before_counts_toward_redundancy() {
        let mut outbox = Outbox::new(3);
        outbox.push(FnDown, "d");
        outbox.push(Other, "o1");
        assert_eq!(outbox.pop(), Some("d"));
        assert_eq!(outbox.pop(), Some("o1"));
        outbox.push(Other, "o2");
        outbox.push(Other, "o3");
        outbox.push(FnUp, "u");
        outbox.push(Other, "after");
        assert_eq!(drain(&mut outbox), vec!["o3", "u", "after"]);
    }

    /// Without a redundant key, the oldest whole press goes: down, the key in it, up, and the
    /// key after it, but not a status line in between.
    #[test]
    fn then_the_oldest_whole_press_goes_together() {
        let mut outbox = filled(
            7,
            &[
                (FnDown, "d1"),
                (Other, "o1"),
                (LineKind::Permission, "perm"),
                (FnUp, "u1"),
                (Other, "after1"),
                (FnDown, "d2"),
                (FnUp, "u2"),
            ],
        );
        outbox.push(FnDown, "d3");
        assert_eq!(drain(&mut outbox), vec!["perm", "d2", "u2", "d3"]);
        assert_eq!(outbox.dropped(), 4);
    }

    #[test]
    fn a_superseded_status_line_goes_before_a_press() {
        let mut outbox = filled(
            4,
            &[
                (LineKind::Permission, "denied"),
                (FnDown, "d"),
                (FnUp, "u"),
                (LineKind::Permission, "granted"),
            ],
        );
        outbox.push(LineKind::ListenerLost, "lost");
        assert_eq!(drain(&mut outbox), vec!["d", "u", "granted", "lost"]);
    }

    /// An fn up whose down was written, and the down of a press still held, are never dropped,
    /// nor the first key of either: with nothing else to drop, the queue goes past its bound.
    #[test]
    fn a_held_press_and_the_end_of_a_written_one_are_never_dropped() {
        let mut outbox = Outbox::new(2);
        outbox.push(FnDown, "d1");
        assert_eq!(outbox.pop(), Some("d1"));
        outbox.push(Other, "o1");
        outbox.push(FnUp, "u1");
        outbox.push(FnDown, "d2");
        outbox.push(Other, "o2");
        assert_eq!(drain(&mut outbox), vec!["o1", "u1", "d2", "o2"]);
        assert_eq!(outbox.dropped(), 0);
    }

    /// What the tracker would really report, pushed through a small queue the app reads only
    /// now and then: the queue never passes its bound, the policy has to drop, and what is
    /// written alternates fn down and fn up, so every up the app reads ends a down it read. A
    /// blind gap's reset is in the stream.
    #[test]
    fn a_backlog_keeps_the_bound_and_every_hold_whole() {
        let mut tracker = FnTracker::new();
        let mut outbox = Outbox::new(16);
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut fn_down = false;
        let mut written = Vec::new();
        for at in 0..5_000u64 {
            let signals: Vec<Signal> = match next() % 10 {
                0..=2 => {
                    fn_down = !fn_down;
                    let flags = if fn_down { SECONDARY_FN } else { 0 };
                    tracker.flags_changed(63, flags, at * 10)
                }
                3 => {
                    fn_down = false;
                    tracker.reset_after_blind_gap().into_iter().collect()
                }
                _ => tracker.key_down(at * 10).into_iter().collect(),
            };
            for signal in signals {
                outbox.push(LineKind::of(signal), signal);
                assert!(outbox.len() <= 16, "{}", outbox.len());
            }
            if next() % 50 == 0 {
                written.extend(outbox.pop());
            }
        }
        written.extend(drain(&mut outbox));
        assert!(outbox.dropped() > 0, "the backlog never filled the queue");
        assert_alternates(&written);
    }

    fn assert_alternates(written: &[Signal]) {
        let mut held = false;
        for signal in written {
            match signal {
                Signal::FnDown => {
                    assert!(!held, "a down while one was held: {written:?}");
                    held = true;
                }
                Signal::FnUp => {
                    assert!(held, "an up with no down: {written:?}");
                    held = false;
                }
                Signal::Other => {}
            }
        }
    }

    /// The same, checked over the whole written stream: every up follows a down, and no down
    /// follows a down.
    #[test]
    fn the_written_stream_alternates_down_and_up() {
        let mut tracker = FnTracker::new();
        let mut outbox = Outbox::new(8);
        let mut written = Vec::new();
        let mut fn_down = false;
        for at in 0..2_000u64 {
            let signals: Vec<Signal> = match at % 7 {
                0 | 3 => {
                    fn_down = !fn_down;
                    tracker.flags_changed(63, if fn_down { SECONDARY_FN } else { 0 }, at)
                }
                5 if at % 11 == 0 => {
                    fn_down = false;
                    tracker.reset_after_blind_gap().into_iter().collect()
                }
                _ => tracker.key_down(at).into_iter().collect(),
            };
            for signal in signals {
                outbox.push(LineKind::of(signal), signal);
            }
            if at % 13 == 0 {
                written.extend(outbox.pop());
            }
        }
        written.extend(drain(&mut outbox));
        assert!(outbox.dropped() > 0);
        assert_alternates(&written);
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pipe {
        Stuck,
        Open,
        Closed,
    }

    /// A writer that blocks while stuck, as a pipe the app is not reading does.
    struct StuckPipe {
        gate: std::sync::Arc<(Mutex<Pipe>, Condvar)>,
        written: std::sync::Arc<Mutex<Vec<u8>>>,
    }

    impl Write for StuckPipe {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let (pipe, moved) = &*self.gate;
            let mut pipe = pipe.lock().unwrap();
            while *pipe == Pipe::Stuck {
                pipe = moved.wait(pipe).unwrap();
            }
            if *pipe == Pipe::Closed {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The tap's side never waits on the pipe: with the writer stuck in a write, a burst far
    /// past the bound is queued at once and the queue keeps its bound. Once the pipe moves, the
    /// writer writes the kept lines in order and unchanged: the fn down, the key that spoiled
    /// the press, the newest keys that fit, and the fn up.
    #[test]
    fn a_stuck_pipe_never_holds_up_the_tap() {
        static OUTBOX: SharedOutbox = SharedOutbox::new(8);
        let gate = std::sync::Arc::new((Mutex::new(Pipe::Stuck), Condvar::new()));
        let set = |state: Pipe| {
            *gate.0.lock().unwrap() = state;
            gate.1.notify_all();
        };
        let written = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut pipe = StuckPipe {
            gate: std::sync::Arc::clone(&gate),
            written: std::sync::Arc::clone(&written),
        };
        let writer = std::thread::spawn(move || OUTBOX.write_lines(&mut pipe));

        OUTBOX.push(FnDown, signal_line(Signal::FnDown, 1));
        // The writer takes the fn down and blocks writing it.
        while OUTBOX.snapshot().0 != 0 {
            std::thread::yield_now();
        }
        let started = Instant::now();
        for at in 2..1_000 {
            OUTBOX.push(Other, signal_line(Signal::Other, at));
        }
        OUTBOX.push(FnUp, signal_line(Signal::FnUp, 1_000));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the pushes waited"
        );
        assert_eq!(OUTBOX.snapshot(), (8, 999 - 8));

        set(Pipe::Open);
        assert!(OUTBOX.flush_for(Duration::from_secs(5)));
        let mut expected = vec![
            signal_line(Signal::FnDown, 1),
            signal_line(Signal::Other, 2),
        ];
        expected.extend((994..1_000).map(|at| signal_line(Signal::Other, at)));
        expected.push(signal_line(Signal::FnUp, 1_000));
        let text = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        let expected: String = expected.iter().map(|line| format!("{line}\n")).collect();
        assert_eq!(text, expected);

        // A closed pipe fails the next write, and the writer returns.
        set(Pipe::Closed);
        OUTBOX.push(LineKind::ListenerLost, "x".to_string());
        assert_eq!(
            writer.join().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
}
