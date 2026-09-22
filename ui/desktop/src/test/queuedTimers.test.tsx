import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render } from '@testing-library/react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { describe, expect, it, onTestFinished, vi } from 'vitest';
import { waitForQueuedZeroDelayTimers } from './queuedTimers';

/**
 * The regression guard for the end-of-file timer drain in ./setup.ts.
 *
 * The fix rests on two facts that are not this repo's to keep true, so they are
 * pinned here against the real libraries rather than described:
 *
 *   1. Radix FocusScope runs its unmount work from a ZERO-delay timer, not
 *      synchronously. If Radix ever moves it to a longer delay, the drain stops
 *      covering it and the CI error in ./queuedTimers.ts can come back — the
 *      FocusScope case below then fails instead.
 *   2. Node runs same-delay timers in the order they were queued, so a timer
 *      queued after the unmount's is ordered after it. That ordering, not a
 *      duration, is what the drain waits on.
 *
 * The wiring is asserted at the source: a wait that is present in this module
 * and absent from setup.ts protects nothing, and no test inside a spec can see
 * setup.ts's `afterAll`, which runs after every `afterEach` and `afterAll` the
 * spec registers. It is not the file's last code, though: vitest runs three
 * things later still. After the file's `afterAll` hooks (@vitest/runner
 * 4.0.18, dist/index.js:1826) come the functions a spec RETURNS from a
 * root-level `beforeAll` (1827-1829), then the teardown of a file-scoped
 * `test.extend` fixture (1830-1833); a WORKER-scoped fixture's teardown runs
 * only once the worker has reported the file finished and the pool has sent
 * "stop", just before jsdom is torn down (onCleanupWorkerContext, run by the
 * "stop" handler in vitest 4.0.18, dist/chunks/init.B6MLFIaN.js:302-304). A
 * probe spec printed its root `afterAll`, its `beforeAll` cleanup, its file
 * fixture's teardown and its worker fixture's teardown in that order, with the
 * timers this drain releases between the first two; a timer queued by any of
 * the last three is not drained, and no spec uses either `beforeAll` cleanups
 * or `.extend` today — see ./queuedTimers.ts.
 */
describe('waitForQueuedZeroDelayTimers', () => {
  it('settles after the zero-delay timers queued ahead of it, and before those behind it', async () => {
    const ran: string[] = [];
    setTimeout(() => ran.push('queued first'), 0);
    setTimeout(() => ran.push('queued second'), 0);

    const drained = waitForQueuedZeroDelayTimers();
    setTimeout(() => ran.push('queued behind the drain'), 0);

    await drained;
    // Ordered, not timed: everything ahead has run, nothing behind has.
    expect(ran).toEqual(['queued first', 'queued second']);
  });

  it('is ordered after the real Radix FocusScope unmount timer that failed CI', async () => {
    const onCloseAutoFocus = vi.fn();
    const { unmount } = render(
      <DialogPrimitive.Root open>
        <DialogPrimitive.Portal>
          <DialogPrimitive.Content onCloseAutoFocus={onCloseAutoFocus}>
            <DialogPrimitive.Title>Title</DialogPrimitive.Title>
            <DialogPrimitive.Description>Description</DialogPrimitive.Description>
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
    );

    // `onCloseAutoFocus` is Radix Dialog's handler for FocusScope's
    // `focusScope.autoFocusOnUnmount` event — the very dispatch that threw on
    // CI (react-focus-scope dist/index.mjs:92). Unmounting only QUEUES it.
    unmount();
    expect(onCloseAutoFocus).not.toHaveBeenCalled();

    // A microtask is not enough: it runs before any queued timer.
    await Promise.resolve();
    expect(onCloseAutoFocus).not.toHaveBeenCalled();

    await waitForQueuedZeroDelayTimers();
    expect(onCloseAutoFocus).toHaveBeenCalledTimes(1);
  });

  it('waits on the real clock, so a spec that left fake timers installed cannot hang afterAll', async () => {
    vi.useFakeTimers();
    // Not a `finally`: if the wait ever lands on the fake clock it never
    // settles, a `finally` never runs, and the fake clock would then hang
    // setup.ts's own afterAll for 30 s on top of this test's clear failure.
    onTestFinished(() => {
      vi.useRealTimers();
    });
    // On a fake clock nobody advances this would never settle; the 2 s budget
    // turns that into this test failing rather than a hook hanging.
    await waitForQueuedZeroDelayTimers();
  }, 2_000);

  it("is awaited first in setup.ts's afterAll, before the file's last network check", () => {
    const setup = readFileSync(join(__dirname, 'setup.ts'), 'utf8');
    // First, so a timer the drain releases that reaches for the network is
    // still reported, by the check that exists for late work like it.
    expect(setup).toMatch(
      /afterAll\(async \(\) => \{\s*await waitForQueuedZeroDelayTimers\(\);\s*assertNoUnexpectedNetworkAttempts\(/
    );
  });
});
