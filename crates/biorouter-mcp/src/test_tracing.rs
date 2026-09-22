//! For test binaries only: make a thread-local (scoped) tracing subscriber
//! see every event its own thread emits, whatever other test threads do.
//!
//! Nothing in production calls this. It is public (and hidden) because the
//! binaries that need it — the `biorouter`, `biorouter-mcp` and
//! `biorouter-server` lib test binaries and two `biorouter` integration
//! binaries — each call it from a `#[ctor]`, and the ones outside this crate
//! link it built without `cfg(test)`.
//!
//! ## The defect this works around, in `tracing-core` 0.1.36
//!
//! Every `tracing` macro caches an `Interest` per callsite, process-wide, and
//! an event is dropped before any subscriber is asked when that cache says
//! `never`. The value is computed when the callsite is first reached, and how
//! it is computed depends on one flag, `has_just_one` (`callsite.rs`,
//! `Dispatchers::rebuilder`):
//!
//! - `false`: it asks every registered dispatcher, under a read lock that
//!   serializes it against `Dispatch::new`'s own recomputation, so a live
//!   capture is always consulted;
//! - `true`: the `Rebuilder::JustOne` fast path asks only
//!   `dispatcher::get_default` — the default of **the thread that happened to
//!   reach the callsite first**, with no lock.
//!
//! ⚠ The flag is not a live count. It starts `true` and is written in exactly
//! one place, `Dispatchers::register_dispatch` — that is, by `Dispatch::new` —
//! as "at most one dispatcher is registered and alive, counting the new one",
//! after pruning the dead. Dropping a dispatcher writes nothing: the flag keeps
//! the value the last registration gave it until the next registration.
//!
//! A test's `with_default` / `set_default` / `with_subscriber` is a scoped
//! dispatcher: registered process-wide, but the default only on its own
//! thread. So when it registers as the only live one, a sibling test thread
//! that is first to reach a callsite the capture is waiting for computes the
//! interest from ITS default — the global no-op — and caches `never`. The
//! capture's own event at that callsite is then dropped at the
//! `interest.is_never()` check, and nothing afterwards repairs it: `Dispatch::new`
//! recomputes every callsite, but the fast path holds no lock, so a
//! computation that started before the capture registered can finish after
//! it. `rebuild_interest_cache()` does not close that either, for the same
//! reason, and takes the fast path itself.
//!
//! Measured 2026-09-21, macOS, on a loaded host, on `providers::utils::tests::
//! private_error_logging_http_trace_excludes_response_payload` ("the control
//! warning must be captured"; reported at 1 and 2 in 200 whole-binary runs by
//! the review that found it):
//!
//! - unforced, `providers::utils::` run 3000 times six at a time: 25 failures,
//!   every one with that message; with the dispatchers below, 0 of 3000;
//! - forced, a probe making a spawned thread the first to reach that `warn!`
//!   inside the test's `with_default`: 5 of 5 failures; the same probe with a
//!   second live dispatcher registered first, 5 of 5 passes; the probe with the
//!   dispatchers below, 5 of 5 passes.
//!
//! ## What this does
//!
//! It registers two dispatchers that never enable anything, and keeps them
//! for the life of the process. The second one's registration writes
//! `has_just_one = false`, and no later registration can write `true` again,
//! because both stay alive and every later one counts at least three — so the
//! fast path is never taken from then on, and every interest is computed over
//! every live dispatcher, including a capture on another thread.
//!
//! It has to be two. With one, its own registration leaves the flag `true`
//! (one alive), and it stays `true` until the FIRST capture registers and
//! counts two: that first capture's start is exactly the window above, a
//! fast-path computation already in flight finishing after the capture's
//! recomputation. Every later registration counts at least two, so only that
//! first one would stay open — but one is enough to lose an event. And it has
//! to run before `main`, because a lazy call can itself race a fast-path
//! computation already in flight.
//!
//! They change nothing else: neither is anyone's default, so no event is
//! delivered to them; they answer `Interest::never()`, which folds into
//! `sometimes` beside a subscriber that wants the event, so the event is then
//! decided by the emitting thread's own subscriber; and their level hint is
//! `OFF`, so the global maximum level stays the maximum of the real
//! subscribers, exactly as without them.

use std::sync::OnceLock;

use tracing::level_filters::LevelFilter;
use tracing::subscriber::Interest;
use tracing::{span, Dispatch, Event, Metadata, Subscriber};

/// A subscriber that enables nothing and asks for nothing.
struct Inert;

impl Subscriber for Inert {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::never()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::OFF)
    }

    fn enabled(&self, _: &Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(0xDEAD)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}

/// Held by a `static`, so never dropped: a dispatcher is registered only as
/// long as something holds it.
static INERT_DISPATCHERS: OnceLock<[Dispatch; 2]> = OnceLock::new();

/// Register the two inert dispatchers (module docs). Idempotent. Call it from
/// a `#[ctor]`, so it runs before any test thread exists.
#[doc(hidden)]
pub fn register_inert_dispatchers() {
    INERT_DISPATCHERS.get_or_init(|| [Dispatch::new(Inert), Dispatch::new(Inert)]);
}

/// Whether [`register_inert_dispatchers`] has run in this process, read
/// without running it — for each binary's guard that its ctor still calls it.
#[doc(hidden)]
pub fn inert_dispatchers_registered() -> bool {
    INERT_DISPATCHERS.get().is_some()
}

/// This crate's own lib test binary, which holds one of the captures
/// (`knowledge::service`'s unreadable-manifest warning).
#[cfg(test)]
static REGISTERED_BEFORE_MAIN: OnceLock<bool> = OnceLock::new();

#[cfg(test)]
#[ctor::ctor]
fn register_before_main_in_this_crates_test_binary() {
    register_inert_dispatchers();
    let _ = REGISTERED_BEFORE_MAIN.set(inert_dispatchers_registered());
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_inert_dispatchers_are_registered_before_main() {
        assert_eq!(
            super::REGISTERED_BEFORE_MAIN.get(),
            Some(&true),
            "the ctor did not register the inert tracing dispatchers before main, so a \
             capture in this binary can miss its own event whenever a sibling test \
             thread reaches that callsite first (module docs)."
        );
    }
}
