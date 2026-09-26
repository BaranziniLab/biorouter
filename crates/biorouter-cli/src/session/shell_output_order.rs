//! Put a shell command's live output back in the order the command printed it.
//!
//! The developer extension streams every non-blank line a shell command prints
//! as a `shell_output` logging notification, and numbers them: `seq` counts one
//! command's streamed lines from 0. The numbering exists because the order is
//! lost on the way here. rmcp hands every incoming notification to a task of
//! its own (`rmcp-0.14.0/src/service.rs`, `tokio::spawn(... handle_notification
//! ...)`), so two adjacent lines race each other into the agent's channel and
//! regularly arrive swapped. The e11 self-test printed `crew --help` with ten
//! adjacent pairs swapped while the recorded tool result, which never passes
//! through that path, was byte-identical to the command's own output.
//!
//! [`ShellOutputOrder`] is the display side of the fix: it releases each tool
//! call's lines strictly in `seq` order and holds a line that arrives ahead of
//! a missing one. It never loses a line — every line it accepts is released
//! exactly once — and it never holds one for long:
//!
//! - the tool call's response releases whatever that call still holds
//!   ([`ShellOutputOrder::finish`]), and the end of the turn releases the rest
//!   ([`ShellOutputOrder::finish_all`]);
//! - a gap is given up on after [`GAP_GRACE`] or once [`MAX_HELD`] lines wait
//!   behind it, because a gap is not always a reordering. The agent's channel
//!   drops a notification when it is full (`try_send`), so a burst can lose a
//!   line outright, and waiting for it would freeze the live view until the
//!   command ends. A line that turns up after its gap was given up on is
//!   printed when it arrives, out of order, rather than dropped;
//! - a line with no `seq` (a server from before the numbering) is released at
//!   once, exactly as it always was.
//!
//! Lines are keyed by the tool request id the agent attaches to each
//! notification, so two commands running at once never hold each other up.
//! This is a display concern only: the tool result the model reads is built
//! from the command's own output and never passes through here.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long a line may wait for an earlier one before the gap is given up on.
///
/// The reordering this undoes is two tasks racing for the same lock, which
/// resolves in microseconds; a line still missing after this long was dropped
/// rather than delayed.
pub const GAP_GRACE: Duration = Duration::from_millis(250);

/// How many lines one tool call may hold behind a gap before the gap is given
/// up on. A reordering displaces a line by one or two places; this many lines
/// arriving past a hole means the hole is a loss.
pub const MAX_HELD: usize = 64;

/// Reorders live `shell_output` lines by their `seq`, per tool call.
#[derive(Debug)]
pub struct ShellOutputOrder {
    /// One entry per tool call with lines in flight, in first-seen order so
    /// [`Self::finish_all`] releases calls in the order they started talking.
    streams: Vec<Stream>,
    grace: Duration,
    max_held: usize,
}

#[derive(Debug)]
struct Stream {
    key: String,
    /// The `seq` that may be released next.
    next: u64,
    held: BTreeMap<u64, Held>,
}

#[derive(Debug)]
struct Held {
    line: String,
    since: Instant,
}

impl Default for ShellOutputOrder {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellOutputOrder {
    pub fn new() -> Self {
        Self::with_limits(GAP_GRACE, MAX_HELD)
    }

    pub fn with_limits(grace: Duration, max_held: usize) -> Self {
        Self {
            streams: Vec::new(),
            grace,
            max_held: max_held.max(1),
        }
    }

    /// Accept one line of tool call `key`'s output and return the lines that
    /// may be printed now, in order. Empty when the line has to wait.
    pub fn accept(
        &mut self,
        key: &str,
        seq: Option<u64>,
        line: String,
        now: Instant,
    ) -> Vec<String> {
        let Some(seq) = seq else {
            return vec![line];
        };
        let grace = self.grace;
        let max_held = self.max_held;
        let stream = self.stream_mut(key);
        let mut ready = Vec::new();

        if seq < stream.next {
            if seq != 0 {
                // Its gap was already given up on: late, but not lost.
                ready.push(line);
                return ready;
            }
            // A second `seq` 0 under one key is a second command (a tool that
            // ran two). Whatever the first still holds comes out first.
            stream.release_all(&mut ready);
            stream.next = 0;
        }
        if stream.held.contains_key(&seq) {
            // Two lines claiming one place can only come from two commands;
            // print it rather than lose either.
            ready.push(line);
            return ready;
        }

        stream.held.insert(seq, Held { line, since: now });
        stream.release_ready(&mut ready);
        while stream.held.len() > max_held {
            stream.skip_gap(&mut ready);
        }
        stream.release_due(now, grace, &mut ready);
        ready
    }

    /// Release the lines whose gap has been open for [`GAP_GRACE`] as of
    /// `now`, across every tool call.
    pub fn release_due(&mut self, now: Instant) -> Vec<String> {
        let grace = self.grace;
        let mut ready = Vec::new();
        for stream in &mut self.streams {
            stream.release_due(now, grace, &mut ready);
        }
        ready
    }

    /// When [`Self::release_due`] next has something to release, if anything
    /// is held at all.
    pub fn next_release_at(&self) -> Option<Instant> {
        self.streams
            .iter()
            .filter_map(Stream::oldest_since)
            .min()
            .map(|since| since + self.grace)
    }

    /// Tool call `key` has answered: release everything it still holds, in
    /// `seq` order, and forget it.
    pub fn finish(&mut self, key: &str) -> Vec<String> {
        let mut ready = Vec::new();
        if let Some(index) = self.streams.iter().position(|s| s.key == key) {
            self.streams.remove(index).release_all(&mut ready);
        }
        ready
    }

    /// The turn is over: release everything still held, one tool call at a
    /// time and each in `seq` order.
    pub fn finish_all(&mut self) -> Vec<String> {
        let mut ready = Vec::new();
        for mut stream in self.streams.drain(..) {
            stream.release_all(&mut ready);
        }
        ready
    }

    fn stream_mut(&mut self, key: &str) -> &mut Stream {
        let index = match self.streams.iter().position(|s| s.key == key) {
            Some(index) => index,
            None => {
                self.streams.push(Stream {
                    key: key.to_string(),
                    next: 0,
                    held: BTreeMap::new(),
                });
                self.streams.len() - 1
            }
        };
        &mut self.streams[index]
    }
}

impl Stream {
    /// Release the contiguous run starting at `next`.
    fn release_ready(&mut self, ready: &mut Vec<String>) {
        while let Some(held) = self.held.remove(&self.next) {
            ready.push(held.line);
            self.next = self.next.saturating_add(1);
        }
    }

    /// Give up on the gap before the lowest held line, and release from there.
    fn skip_gap(&mut self, ready: &mut Vec<String>) {
        if let Some((&lowest, _)) = self.held.first_key_value() {
            self.next = lowest;
            self.release_ready(ready);
        }
    }

    /// Give up on gaps for as long as the longest-waiting line has waited out
    /// `grace`.
    fn release_due(&mut self, now: Instant, grace: Duration, ready: &mut Vec<String>) {
        while let Some(since) = self.oldest_since() {
            if now < since + grace {
                break;
            }
            self.skip_gap(ready);
        }
    }

    /// Release everything held, in `seq` order, gaps and all.
    fn release_all(&mut self, ready: &mut Vec<String>) {
        let held = std::mem::take(&mut self.held);
        ready.extend(held.into_values().map(|held| held.line));
    }

    fn oldest_since(&self) -> Option<Instant> {
        self.held.values().map(|held| held.since).min()
    }
}

/// Resolve at `at`, or never when nothing is held. For a `select!` arm that
/// calls [`ShellOutputOrder::release_due`].
pub async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "call_1";

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Feed `(seq, line)` pairs in arrival order, all at one instant, and
    /// collect what is printed.
    fn feed(order: &mut ShellOutputOrder, key: &str, arrivals: &[(u64, &str)]) -> Vec<String> {
        let now = Instant::now();
        let mut printed = Vec::new();
        for (seq, line) in arrivals {
            printed.extend(order.accept(key, Some(*seq), line.to_string(), now));
        }
        printed
    }

    #[test]
    fn shuffled_arrivals_print_in_seq_order() {
        let mut order = ShellOutputOrder::new();
        let printed = feed(
            &mut order,
            KEY,
            &[(1, "b"), (0, "a"), (3, "d"), (2, "c"), (5, "f"), (4, "e")],
        );
        assert_eq!(printed, lines(&["a", "b", "c", "d", "e", "f"]));
        assert_eq!(order.next_release_at(), None, "nothing left held");
    }

    #[test]
    fn a_gap_is_held_then_released_when_it_fills() {
        let mut order = ShellOutputOrder::new();
        let now = Instant::now();
        assert_eq!(order.accept(KEY, Some(0), "a".into(), now), lines(&["a"]));
        assert!(order.accept(KEY, Some(2), "c".into(), now).is_empty());
        assert!(order.accept(KEY, Some(3), "d".into(), now).is_empty());
        assert!(order.next_release_at().is_some(), "c and d wait for b");
        assert_eq!(
            order.accept(KEY, Some(1), "b".into(), now),
            lines(&["b", "c", "d"])
        );
        assert_eq!(order.next_release_at(), None);
    }

    #[test]
    fn the_tool_response_releases_the_rest_in_order() {
        let mut order = ShellOutputOrder::new();
        let printed = feed(&mut order, KEY, &[(0, "a"), (3, "d"), (2, "c"), (5, "f")]);
        assert_eq!(printed, lines(&["a"]));
        assert_eq!(order.finish(KEY), lines(&["c", "d", "f"]));
        assert_eq!(order.next_release_at(), None);
        assert!(order.finish(KEY).is_empty(), "released exactly once");
    }

    #[test]
    fn the_end_of_the_turn_releases_every_call_in_first_seen_order() {
        let mut order = ShellOutputOrder::new();
        let now = Instant::now();
        assert!(order.accept("b", Some(2), "b2".into(), now).is_empty());
        assert!(order.accept("a", Some(1), "a1".into(), now).is_empty());
        assert!(order.accept("b", Some(1), "b1".into(), now).is_empty());
        assert_eq!(order.finish_all(), lines(&["b1", "b2", "a1"]));
        assert_eq!(order.next_release_at(), None);
    }

    #[test]
    fn lines_without_seq_pass_straight_through() {
        let mut order = ShellOutputOrder::new();
        let now = Instant::now();
        assert_eq!(
            order.accept(KEY, None, "old server".into(), now),
            lines(&["old server"])
        );
        // Even while the same call holds numbered lines behind a gap.
        assert!(order.accept(KEY, Some(1), "held".into(), now).is_empty());
        assert_eq!(
            order.accept(KEY, None, "still".into(), now),
            lines(&["still"])
        );
        assert_eq!(order.finish(KEY), lines(&["held"]));
    }

    #[test]
    fn separate_commands_do_not_interleave_or_hold_each_other_up() {
        let mut order = ShellOutputOrder::new();
        let now = Instant::now();
        // A's first line is late; B is not held behind A's gap.
        assert!(order.accept("A", Some(1), "A1".into(), now).is_empty());
        assert_eq!(order.accept("B", Some(0), "B0".into(), now), lines(&["B0"]));
        assert_eq!(order.accept("B", Some(1), "B1".into(), now), lines(&["B1"]));
        assert_eq!(
            order.accept("A", Some(0), "A0".into(), now),
            lines(&["A0", "A1"])
        );
        // Each call's seq is its own: B's seq 2 is not A's.
        assert!(order.accept("A", Some(3), "A3".into(), now).is_empty());
        assert_eq!(order.accept("B", Some(2), "B2".into(), now), lines(&["B2"]));
        // Finishing B releases nothing of A's.
        assert!(order.finish("B").is_empty());
        assert_eq!(
            order.accept("A", Some(2), "A2".into(), now),
            lines(&["A2", "A3"])
        );
    }

    #[test]
    fn a_gap_is_given_up_on_after_the_grace_period() {
        let grace = Duration::from_millis(250);
        let mut order = ShellOutputOrder::with_limits(grace, 64);
        let t0 = Instant::now();
        assert_eq!(order.accept(KEY, Some(0), "a".into(), t0), lines(&["a"]));
        // `b` (seq 1) was dropped by a full channel and never arrives.
        assert!(order.accept(KEY, Some(2), "c".into(), t0).is_empty());
        let t1 = t0 + Duration::from_millis(100);
        assert!(order.accept(KEY, Some(3), "d".into(), t1).is_empty());
        assert_eq!(order.next_release_at(), Some(t0 + grace));
        assert!(order
            .release_due(t0 + grace - Duration::from_millis(1))
            .is_empty());
        assert_eq!(order.release_due(t0 + grace), lines(&["c", "d"]));
        assert_eq!(order.next_release_at(), None);
        // The live view moves on in order after the loss.
        assert_eq!(
            order.accept(KEY, Some(4), "e".into(), t1 + grace),
            lines(&["e"])
        );
        // And a line that turns up after its gap was given up on still prints.
        assert_eq!(
            order.accept(KEY, Some(1), "b".into(), t1 + grace),
            lines(&["b"])
        );
    }

    #[test]
    fn an_arrival_releases_a_gap_that_has_already_waited_out_its_grace() {
        let grace = Duration::from_millis(250);
        let mut order = ShellOutputOrder::with_limits(grace, 64);
        let t0 = Instant::now();
        assert!(order.accept(KEY, Some(1), "b".into(), t0).is_empty());
        assert_eq!(
            order.accept(KEY, Some(3), "d".into(), t0 + grace),
            lines(&["b"]),
            "b waited out its grace; d is still inside its own"
        );
        assert_eq!(order.release_due(t0 + grace * 2), lines(&["d"]));
    }

    #[test]
    fn a_gap_is_given_up_on_once_too_many_lines_wait_behind_it() {
        let mut order = ShellOutputOrder::with_limits(Duration::from_secs(3600), 3);
        let now = Instant::now();
        assert_eq!(order.accept(KEY, Some(0), "a".into(), now), lines(&["a"]));
        // seq 1 is lost; 2, 3 and 4 wait.
        assert!(order.accept(KEY, Some(3), "d".into(), now).is_empty());
        assert!(order.accept(KEY, Some(2), "c".into(), now).is_empty());
        assert!(order.accept(KEY, Some(4), "e".into(), now).is_empty());
        // The fourth held line is one too many.
        assert_eq!(
            order.accept(KEY, Some(6), "g".into(), now),
            lines(&["c", "d", "e"])
        );
        assert_eq!(
            order.accept(KEY, Some(5), "f".into(), now),
            lines(&["f", "g"])
        );
    }

    #[test]
    fn a_second_command_under_one_call_starts_its_own_sequence() {
        let mut order = ShellOutputOrder::new();
        let printed = feed(
            &mut order,
            KEY,
            &[
                (0, "one-a"),
                (1, "one-b"),
                (3, "one-d"),
                (0, "two-a"),
                (1, "two-b"),
            ],
        );
        assert_eq!(
            printed,
            lines(&["one-a", "one-b", "one-d", "two-a", "two-b"])
        );
        assert_eq!(order.next_release_at(), None);
    }

    #[test]
    fn a_duplicate_place_is_printed_not_lost() {
        let mut order = ShellOutputOrder::new();
        let printed = feed(&mut order, KEY, &[(2, "x"), (2, "y"), (0, "a"), (1, "b")]);
        assert_eq!(printed, lines(&["y", "a", "b", "x"]));
    }

    #[test]
    fn nothing_is_held_means_no_deadline() {
        let order = ShellOutputOrder::new();
        assert_eq!(order.next_release_at(), None);
    }

    /// `biorouter crew --help` as the e11 self-test's direct capture printed it
    /// (`direct-rerun/cli-crew-help.stdout`), blank lines removed — the lines
    /// the developer extension streams, whose `seq` is their index here. Trailing
    /// spaces are clap's and part of the expected bytes.
    const E11_CREW_HELP: [&str; 49] = [
        "Collaborate through Crew using the shared local daemon",
        "Usage: biorouter crew [OPTIONS] <COMMAND>",
        "Commands:",
        "  daemon         ",
        "  credentials    ",
        "  status         Show saved connections and their daemon-reported state",
        "  connections    ",
        "  auth           Authenticate SSH through the shared daemon's owned authentication session",
        "  connect        Open the verified SSH bridge for the selected connection",
        "  disconnect     Close the selected SSH connection",
        "  workspace      ",
        "  enroll         ",
        "  members        List the principals visible in the selected workspace",
        "  teams          ",
        "  channels       ",
        "  invites        ",
        "  profile        ",
        "  ownership      ",
        "  remove-member  Remove a member from a channel owned by you",
        "  history        Read a page of authorized channel messages",
        "  search         Search authorized channel messages",
        "  watch          Follow channel messages. Ctrl-C detaches without cancelling tasks",
        "  send           Post as your own authenticated workspace identity",
        "  context        Show the channel/context scope of an existing grant",
        "  files          ",
        "  tasks          ",
        "  grants         ",
        "  privacy        ",
        "  help           Print this message or the help of the given subcommand(s)",
        "Options:",
        "      --connection <CONNECTION>",
        "          Saved connection ID. Required when more than one connection is saved",
        "      --expected-mode <EXPECTED_MODE>",
        "          Require this privacy mode for send, task start, grants, and file upload/download/resume",
        "          [possible values: private, public]",
        "      --expected-policy-epoch <EXPECTED_POLICY_EPOCH>",
        "          Require this verified connection policy epoch when starting tasks or granting access",
        "      --expected-workspace-policy-epoch <EXPECTED_WORKSPACE_POLICY_EPOCH>",
        "          Require this verified workspace policy epoch when starting tasks or granting access",
        "      --no-start",
        "          Require an already-running shared daemon",
        "      --approval-key-stdin",
        "          Read the human approval key from stdin's first line instead of a hidden prompt",
        "      --output-format <OUTPUT_FORMAT>",
        "          [default: text] [possible values: text, json, stream-json]",
        "      --request-id <REQUEST_ID>",
        "          Reuse this identifier when retrying the same mutation after an uncertain result",
        "  -h, --help",
        "          Print help",
    ];

    /// The order those lines reached the e11 terminal (`run.stdout.log`), as
    /// indices into [`E11_CREW_HELP`]: ten adjacent pairs swapped.
    const E11_ARRIVAL_ORDER: [u64; 49] = [
        0, 1, 3, 2, 4, 5, 6, 7, 8, 9, 11, 10, 12, 13, 14, 15, 16, 18, 17, 19, 20, 22, 21, 23, 24,
        26, 25, 27, 28, 29, 30, 31, 32, 34, 33, 36, 35, 37, 39, 38, 40, 41, 43, 42, 44, 45, 46, 47,
        48,
    ];

    #[test]
    fn e11_crew_help_renders_as_the_direct_capture() {
        // The fixture really is the observed defect: the same lines, swapped.
        let mut sorted = E11_ARRIVAL_ORDER.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..49).collect::<Vec<u64>>());
        assert_ne!(E11_ARRIVAL_ORDER.to_vec(), sorted);

        let mut order = ShellOutputOrder::new();
        let now = Instant::now();
        let mut printed = Vec::new();
        for seq in E11_ARRIVAL_ORDER {
            let line = E11_CREW_HELP[seq as usize].to_string();
            printed.extend(order.accept("call_crew_help", Some(seq), line, now));
        }
        // The swaps resolve as they arrive; nothing waits for the response.
        assert_eq!(order.next_release_at(), None);
        assert!(order.finish("call_crew_help").is_empty());

        let rendered = printed.join("\n") + "\n";
        let direct = E11_CREW_HELP.join("\n") + "\n";
        assert_eq!(
            rendered, direct,
            "identical to the direct capture, blank lines aside"
        );
    }
}
