/**
 * Let the zero-delay timers a spec file has already queued run while jsdom is
 * still installed. Awaited once per file, from `afterAll` in ./setup.ts.
 *
 * ## The failure this closes
 *
 * frontend.yml "Unit tests (vitest)", run 35464728334 attempt 1, job
 * 105954881938 (2026-09-19): `Test Files 534 passed`, `Tests 6153 passed`,
 * `Errors 1 error`, exit 1 —
 *
 *   TypeError: Failed to execute 'dispatchEvent' on 'EventTarget': parameter 1
 *   is not of type 'Event'.
 *    ❯ HTMLDivElement.dispatchEvent jsdom/lib/jsdom/living/generated/EventTarget.js:236
 *    ❯ Timeout._onTimeout @radix-ui/react-focus-scope/dist/index.mjs:92
 *   This error originated in "…/BottomMenuExtensionSelection.privacy.test.tsx"
 *
 * Every assertion passed. The mechanism:
 *
 *   1. Radix `FocusScope` (inside every Dialog, Sheet, AlertDialog, Popover,
 *      Select and DropdownMenu content) restores focus on unmount from a REAL
 *      `setTimeout(…, 0)` scheduled in its effect cleanup
 *      (react-focus-scope 1.1.7, dist/index.mjs:89). The callback builds
 *      `new CustomEvent(…)` from the GLOBAL `CustomEvent` and dispatches it on
 *      the scope's container (index.mjs:90-92).
 *   2. Testing Library's `cleanup()` is the unmount — setup.ts's `afterEach`,
 *      or a spec's own — and a test body can unmount too. With `bail` unset,
 *      as it is here, nothing vitest runs between hooks or tests gives a
 *      queued timer a turn; only a hook or test that itself waits on a
 *      macrotask does (measured: a probe spec's body timer outlived every
 *      hook after it). That holds only while `bail` stays unset: set it
 *      (`test.bail` in vitest.config.ts, or `--bail` on frontend.yml's
 *      `npx vitest run`) and a FAILED test's `onAfterRunTask` awaits an IPC
 *      round trip, `rpc().getCountOfFailedTests()` (vitest 4.0.18,
 *      dist/chunks/index.6Qv1eEA6.js:98-99), which does give it one. So the
 *      LAST test's timer is still queued when the file's hooks end unless
 *      something after it waits, and in a spec whose tests never wait, so is
 *      every earlier test's (census below).
 *   3. When the worker reports the file finished, the pool sends "stop" and
 *      vitest's jsdom teardown deletes the window globals and puts Node's own
 *      `CustomEvent` back (vitest 4.0.18, dist/chunks/index.CyBMJtT7.js:529-535).
 *      `setTimeout` is not one of the keys it swaps, so the timer is Node's and
 *      survives teardown. If it runs after it, the event is Node's and jsdom's
 *      `dispatchEvent` rejects it as not an `Event`. vitest reports that as an
 *      unhandled error and exits 1.
 *
 * Step 3 is a race between that timer and the "stop" round trip. A worker that
 * wakes from its I/O poll with "stop" readable AND the timer expired runs the
 * I/O callback first (libuv runs due timers in a later phase), so a worker
 * descheduled across the timer's 1 ms deadline tears jsdom down first. Losing
 * takes two things in turn: IPC round trips FAST enough that the timer is still
 * queued when the worker posts `testfileFinished`, and THEN "stop" readable
 * before the worker is back at its timers, which a worker descheduled across
 * the deadline guarantees. Load supplies the second and takes away the first
 * (the second bullet below), so a loaded machine is not simply likelier to
 * lose. The CI failure shows a runner can supply both; how often one does was
 * never measured there. Measured on this repo's tree (537 files):
 *
 *   - 50 spec files end with this timer queued by their last test's cleanup
 *     (a probe wrapping `setTimeout` in the fork workers); 75 queue it from
 *     some `afterEach`. BottomMenuExtensionSelection.privacy.test.tsx is one
 *     of the 50, and 14 of the 50 call `cleanup()` from their own `afterEach`,
 *     which vitest runs BEFORE setup.ts's.
 *   - Whether the timer is still queued at the instant the worker posts
 *     `testfileFinished` depends on the machine at the time, not on the file.
 *     Between the file's last hook and that post the worker awaits IPC round
 *     trips to the pool — `snapshotSaved` (vitest 4.0.18,
 *     dist/chunks/test.B8ej_ZHS.js:167), then `finishSendTasksUpdate`
 *     (@vitest/runner 4.0.18, dist/index.js:1942) — and a timer whose 1 ms is
 *     up by then runs inside them, while jsdom is still installed (a trace of
 *     6 runs of that file alone, pre-fix: it ran during `snapshotSaved` in all
 *     6). So the counts move, and in one direction: the slower those round
 *     trips — the busier the machine — the more often the timer runs inside
 *     them, harmlessly, and the less often it is still queued at the post.
 *     The vulnerable state needs fast round trips first and "stop" winning
 *     the worker's next turn after (above), so a busy machine can show 0
 *     reproductions while the race is still there. On the pre-fix tree the
 *     first probe saw the timer still queued at that post in 11 of 15 runs
 *     of the file alone (its load average is not in the record); re-run later
 *     the same day on the same tree, at a load average of 20 to 25, the same
 *     probe saw 4, 0 and 1 of 15, and a heavier probe (a stack captured per
 *     timer), run on the fixed tree with the drain defeated, 0 of 15. Beside
 *     a slower second file, a worker preload (NODE_OPTIONS=--import) that
 *     makes the worker lose the race whenever the timer is still queued at
 *     that instant — it holds the callback until the worker is about to post
 *     "stopped", i.e. after teardown — found it queued in 8 of 30 runs and
 *     reproduced the CI error in all 8.
 *   - The same preload over the whole suite failed it exactly as CI did —
 *     `537 passed`, `Errors 1 error`, exit 1 — from a DIFFERENT one of the 50
 *     (SessionListView.declassify.test.tsx). With this wait in place, 0 of 538
 *     files finished with the timer queued in that run, nor the file in any of
 *     120 provoked runs of the two-file pair or 30 runs of it alone.
 *   - Unprovoked, the race was never lost locally (0 of 60 runs, 12 in
 *     parallel — the busy-machine case above, so that 0 is not evidence of
 *     absence). It is rare, it is real, and it can hit any of the 50.
 *   - The census of 50 counted only timers queued from the last test's
 *     after-hooks, so it could not see one queued in a test BODY, nor one an
 *     earlier test left queued. A second census (2026-09-21: 538 files, 527
 *     on jsdom; a record-only worker preload listing every timer still queued
 *     at the instant this drain is queued, with the phase and the test that
 *     queued it) gave the same answer in three runs: 60 files end with a
 *     FocusScope unmount timer queued. 57 have one queued after a test body
 *     (`afterEach` and the cleanups after it); 5 have one queued inside a
 *     body or `beforeEach`, 3 of them nothing else, so the first census
 *     missed them; 11 carry timers from more than one test — in
 *     BottomMenuReasoningEffort.test.tsx, whose tests are all synchronous,
 *     from three. Each is ahead of the drain in Node's list, wherever it was
 *     queued, so the drain covers them all. 16 of the 60 call `cleanup()`
 *     from their own `afterEach` — `afterEach(cleanup)`, or an `afterEach`
 *     callback that calls it (a TypeScript AST scan of those 60 files; the
 *     same scan finds the 14 among the first census's 50).
 *   - ⚠ A loop over ONE spec file cannot show this failure even when it
 *     happens: vitest prints its summary when the last file finishes, and an
 *     error the worker reports after that is dropped (measured: the callback
 *     threw, the run still exited 0). Reproduce beside a slower second file.
 *
 * ## Why this is a wait on a real signal and not a sleep
 *
 * The promise resolves from a `setTimeout(…, 0)` queued AFTER everything the
 * file's unmounts queued. Node coerces a 0 delay to 1 ms and keeps timers of
 * equal duration in one list in insertion order, running a list from its head
 * and never past a timer that is not yet due (lib/internal/timers.js). So this
 * callback cannot run before any zero-delay timer queued ahead of it: its
 * firing IS the evidence they have run. No duration is guessed.
 * queuedTimers.test.tsx pins that ordering against the real Radix timer.
 *
 * The rules that follow from it, each deliberate:
 *
 *   - In setup.ts, not in the specs. vitest runs a suite's after-hooks in
 *     reverse registration order (`sequence.hooks: 'stack'`, its default,
 *     which vitest.config.ts does not override), and setup.ts registers its
 *     hooks before the spec is imported (@vitest/runner 4.0.18,
 *     dist/index.js:1372-1385). So setup.ts's `afterAll` runs after every
 *     `afterEach`, every describe block's hooks and every root-level
 *     `afterAll` a spec registers: one wait there covers the 16 of the 60
 *     that call `cleanup()` from their own `afterEach`, and every spec
 *     written later. It is the file's last HOOK, not its last code — see the
 *     first item under "What it does NOT cover". Wrapping setup.ts's own
 *     `cleanup()` in fake timers would not do: by then a spec that unmounts
 *     from its own `afterEach` has already queued its timer on the real
 *     clock.
 *   - `afterAll`, not `afterEach`. Only a timer still queued when the file ends
 *     can outlive jsdom, and every zero-delay timer queued before the drain —
 *     by any test, in a body or a hook — is ahead of it in Node's list. So one
 *     wait per file covers them all, and a wait per test would add nothing.
 *     `afterAll` also comes after a spec's own root-level `afterAll`, which an
 *     `afterEach` never does.
 *   - A `setTimeout`, not `setImmediate` or a microtask. Both of those can run
 *     BEFORE a 1 ms timer that is already queued, which would wait for nothing.
 *   - The REAL `setTimeout`, captured when setup.ts first imports this module
 *     (before any spec body runs). A spec that leaves `vi.useFakeTimers()`
 *     installed would otherwise make this promise wait on a fake clock nobody
 *     advances, and hang `afterAll`. The unmount timer in that spec is itself
 *     a fake, so it cannot outlive jsdom and there is nothing to wait for.
 *
 * What it does NOT cover, and what was measured about each:
 *
 *   - A timer queued AFTER the drain. Three things run later than setup.ts's
 *     `afterAll`. In the file's own suite, straight after its `afterAll`
 *     hooks (@vitest/runner 4.0.18, dist/index.js:1826), vitest runs the
 *     functions RETURNED from root-level `beforeAll` hooks (1827-1829) and
 *     then the teardown of any file-scoped `test.extend` fixture (1830-1833).
 *     A WORKER-scoped fixture (`{ scope: 'worker' }`) is torn down later
 *     still, and not by the file's suite at all: the runner hands its cleanup
 *     to `onCleanupWorkerContext` (dist/index.js:1922-1931; vitest 4.0.18,
 *     dist/chunks/test.B8ej_ZHS.js:141-142), and the worker runs those
 *     cleanups only when the pool sends "stop" — after it has posted
 *     `testfileFinished` — immediately before the jsdom teardown, in the same
 *     message handler (dist/chunks/init.B6MLFIaN.js:302-304: `teardown()`,
 *     which runs them, 130-131 and 73-74, then `workerTeardown`, which is the
 *     jsdom teardown, dist/chunks/base.CJ0Y4ePK.js:129-130). A probe spec
 *     printed its root `afterAll`, then the timers this drain releases, then
 *     its `beforeAll` cleanup, then its file fixture's teardown, then its
 *     worker fixture's teardown, in that order, the last from a stack running
 *     through `teardown` (init.B6MLFIaN.js:131) up to that "stop" handler
 *     (302), not the `finally` of `execute` (119-122). A timer queued in any
 *     of the three is behind this one. One queued in a worker fixture's
 *     teardown is behind the jsdom teardown too: a 1 ms timer queued there
 *     fired with `window` already gone in 3 of 3 runs, and a worker fixture
 *     whose teardown unmounts an open Dialog failed as CI did WITH this wait
 *     in place and nothing forcing it — `Errors 1 error`, exit 1, the same
 *     TypeError from index.mjs:92 — in 19 of 40 runs beside a second file
 *     (two batches of 20: 6 and 13; exit 0 and no error in the rest, because
 *     whether it is reported is a race with "stopped" — see the next item).
 *     The other two are back in the race this closes. Forced to lose it — a
 *     preload holding every non-vitest timer queued after the drain until the
 *     worker posts "stopped" — a root-level `beforeAll` whose cleanup
 *     unmounts an open Dialog failed as CI did WITH this wait in place: `2
 *     passed`, `Errors 1 error`, exit 1, the same TypeError from
 *     index.mjs:92. Unforced, it did not lose in 15 runs on a Mac, which says
 *     nothing about a CI runner. No spec does any of the three today (a
 *     TypeScript AST scan of all 538, 2026-09-21: no `beforeAll` returns
 *     anything, and nothing but `expect` has `.extend` called on it, so there
 *     is no fixture of either scope). Unmount from an `afterAll` instead; that
 *     runs before this.
 *   - A LONGER timer, whoever queued it: the drain is ordered only after
 *     timers of its own duration, 1 ms. With this wait in place, the second
 *     census found non-vitest timers still queued at jsdom teardown in 24 to
 *     26 of the 527 jsdom files, none shorter than 32 ms, and all queued in a
 *     test body or `beforeEach` bar one queued after the drain in one run of
 *     three — the first census, looking only at after-hooks, could not see
 *     them:
 *     chatStreamStore's sleeps and fallbacks (32 ms to 5 s), the sleeps in
 *     catalogSubscription and sessionMetaSubscription, UserMessage's 50 ms
 *     focus, Radix Tooltip's 300 ms delay, @tanstack/pacer throttles. In
 *     after-hooks the first census found one non-zero delay,
 *     SessionListView's 10 ms reveal timer, which its own effect cleanup
 *     clears — that was the earlier CI failure of this same shape (`window is
 *     not defined`), fixed at the component — and the second found none
 *     still queued at the drain.
 *     Such a timer can fire only within its own file's worker lifetime, and
 *     never in a LATER file, for as long as every file gets a worker of its
 *     own — ./vitestIsolation.test.ts fails if the config or any command that
 *     runs vitest changes that. vitest.config.ts sets no `pool` or `isolate`,
 *     so the defaults apply — `forks` (vitest 4.0.18,
 *     dist/chunks/coverage.AVPTjMgw.js:2478) and `isolate: true`
 *     (dist/chunks/defaults.BOqNVLsY.js:39) — and under them the pool builds
 *     a new runner, and so forks a new child process, for every file
 *     (dist/chunks/cli-api.B7PN_QUv.js:8016, 8117, 7669; a runner is shared
 *     only when `isolate` is false, 8057 and 8100). That child's lifetime
 *     ends after `testfileFinished`, in this order: the pool sends "stop"
 *     (8065, 7563-7567) and waits for "stopped" (up to its 60 s STOP_TIMEOUT,
 *     7381 and 7546-7568), stops listening to the child (7571-7573), then
 *     sends it SIGTERM and, if it is still alive 500 ms later, SIGKILL
 *     (7696-7697); under `pool: 'threads'`, which the guard also allows, it
 *     ends the worker thread with `thread.terminate()` instead (7759).
 *     Neither vitest's worker nor this repo's src/ installs a SIGTERM
 *     handler (vitest adds one only under `--prof`-style flags, and it exits
 *     at once, dist/chunks/init-forks._y3TW739.js:11; the dependencies a spec
 *     loads were not audited), so SIGTERM should end it; a handler that kept
 *     it running would stretch the lifetime to the SIGKILL. Measured: 538
 *     files ran in 538 pids. A 5000 ms timer queued as one file's last act
 *     never fired, while the next file, in the same worker slot
 *     (`--maxWorkers=1`), slept 6 s and found the first file's pid already
 *     gone; with `--no-isolate` (the control) the same timer fired 5001 ms
 *     later inside the next file, in the same pid.
 *     How long a child actually runs on is the machine's, not a limit: in one
 *     run across the 538 files (load average about 20) a worker's event loop
 *     ran on for a median 4.6 ms after `testfileFinished` (p95 21.5, max
 *     112.5), and for a median 1.7 ms (max 90) after jsdom teardown began. A
 *     longer timer that comes due before the child dies fires; one due later
 *     never does. One that fires after teardown and before "stopped" is
 *     reported exactly as the CI failure was; after "stopped" its error is
 *     dropped (a control timer that fired after "stopped", 1.5 ms after
 *     teardown began, and threw `document is not defined` left the run at
 *     exit 0). In three full runs (load averages 66 to 177) with a preload
 *     recording every non-vitest timer that fired after teardown, none did.
 *     That is the race this drain closes, over a window such a timer has to
 *     land in by chance, and nothing here closes it for them.
 *   - A timer a drained callback queues in turn: it lands behind this one.
 *     Across the 75 files, FocusScope's callback queued 5, all jsdom's own
 *     `selectionchange` (living/selection/Selection-impl.js:349), which jsdom
 *     builds from its internal `Event` (living/helpers/events.js), not the
 *     global one teardown swaps, so running after teardown cannot break it.
 */
const realSetTimeout: typeof globalThis.setTimeout = globalThis.setTimeout;

export function waitForQueuedZeroDelayTimers(): Promise<void> {
  return new Promise<void>((resolve) => {
    realSetTimeout(resolve, 0);
  });
}
