import { render } from '@testing-library/react';
import { useEffect, useRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

/**
 * The regression guard for the `scrollIntoView` polyfill in ./setup.ts.
 *
 * The bug this replaces was read as flakiness for exactly as long as it took to
 * notice WHERE the throw came from: `commitHookPassiveMountEffects`. Two specs
 * installed the polyfill in `beforeEach` and deleted it in `afterEach`, and a
 * passive effect is not flushed when the test body returns — React commits a
 * render in one scheduler callback and flushes that render's passive effects in
 * the next. vitest's `afterEach` hooks run in reverse registration order, so the
 * spec's teardown removed the property BEFORE setup.ts's `cleanup()` unmounted
 * the tree, and unmounting is what flushes the queue. The effect then threw
 * inside React and failed a test that had already asserted its point.
 *
 * ⚠ This file is written to open that exact window, so it fails if the polyfill
 * ever regains a per-test lifetime. `Deferred` returns one macrotask after
 * render: the re-render its promise triggers is committed by then, its passive
 * effects are not yet flushed, and the flush therefore lands in `cleanup()` —
 * after every `afterEach` this file could possibly register.
 */
function Deferred() {
  const [rows, setRows] = useState<string[]>([]);
  const listRef = useRef<HTMLDivElement>(null);

  // Stands in for a palette's async reads: each resolution re-renders.
  useEffect(() => {
    let alive = true;
    void Promise.resolve(['a', 'b']).then((next) => {
      if (alive) setRows(next);
    });
    return () => {
      alive = false;
    };
  }, []);

  // PASSIVE, exactly like MentionPopover's scroll — not `useLayoutEffect`.
  useEffect(() => {
    const selected = listRef.current?.children[0] as HTMLElement | undefined;
    selected?.scrollIntoView({ block: 'nearest', behavior: 'auto' });
  });

  return (
    <div ref={listRef}>
      {rows.map((row) => (
        <div key={row}>{row}</div>
      ))}
    </div>
  );
}

describe('the process-wide scrollIntoView polyfill', () => {
  it('is installed for every spec, because jsdom implements none', () => {
    expect(typeof Element.prototype.scrollIntoView).toBe('function');
  });

  it('survives a spy that a spec restores', () => {
    const spy = vi.spyOn(Element.prototype, 'scrollIntoView');
    document.createElement('div').scrollIntoView();
    expect(spy).toHaveBeenCalledTimes(1);
    spy.mockRestore();
    // Restoring puts the polyfill back rather than leaving the property gone —
    // which is the whole reason a spec must spy rather than redefine.
    expect(typeof Element.prototype.scrollIntoView).toBe('function');
  });

  it('is still there when a passive effect is flushed by cleanup(), after every afterEach', async () => {
    render(<Deferred />);
    // One macrotask: long enough for the promise's re-render to COMMIT, short
    // enough that its passive effects are still queued when this test returns.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(typeof Element.prototype.scrollIntoView).toBe('function');
    // The assertion that matters is made by the runner, not by this line: if the
    // polyfill has a per-test lifetime, the flush inside cleanup() throws
    // `TypeError: … scrollIntoView is not a function` and this test fails.
  });
});
