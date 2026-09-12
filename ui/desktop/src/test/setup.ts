import '@testing-library/jest-dom';
import { vi, afterEach } from 'vitest';
import { cleanup, configure } from '@testing-library/react';
import { ASYNC_UTIL_TIMEOUT_MS, TEST_TIMEOUT_MS, MIN_TIMEOUT_HEADROOM } from './timeouts';
import { client } from '../api/client.gen';

// This is the standard setup to ensure that React Testing Library's
// automatic cleanup runs after each test.
afterEach(() => {
  cleanup();
});

// Keep routine application logging quiet. Warnings and errors stay connected to
// the test runner so unexpected diagnostics cannot pass invisibly; tests that
// deliberately exercise one must install a local spy.
global.console = {
  ...console,
  log: vi.fn(),
};

// Mock window.navigator.clipboard for copy functionality tests
Object.assign(navigator, {
  clipboard: {
    writeText: vi.fn(() => Promise.resolve()),
  },
});

// JSDOM in this version doesn't provide a usable Storage; install a minimal
// in-memory localStorage so modules that touch it can be unit-tested.
class MemoryStorage implements Storage {
  private store = new Map<string, string>();
  get length() {
    return this.store.size;
  }
  clear(): void {
    this.store.clear();
  }
  getItem(key: string): string | null {
    return this.store.has(key) ? (this.store.get(key) as string) : null;
  }
  key(index: number): string | null {
    return Array.from(this.store.keys())[index] ?? null;
  }
  removeItem(key: string): void {
    this.store.delete(key);
  }
  setItem(key: string, value: string): void {
    this.store.set(key, String(value));
  }
}
const memoryLocal = new MemoryStorage();
Object.defineProperty(globalThis, 'localStorage', {
  value: memoryLocal,
  writable: true,
});

/**
 * Testing Library's async helpers (`findBy*`, `waitFor`) time out on their OWN
 * `asyncUtilTimeout`, which is INDEPENDENT of vitest's `testTimeout`. Both
 * numbers, and the invariant between them, live in ./timeouts.ts — read that
 * file before changing either.
 */
configure({ asyncUtilTimeout: ASYNC_UTIL_TIMEOUT_MS });

// The invariant, enforced rather than documented. If a wait can outlive its
// test, vitest kills the test first and Testing Library's far more useful error
// ("Unable to find an element with the text: …") is never thrown. That is the
// exact collision this repo shipped, and it read as flakiness for weeks.
if (ASYNC_UTIL_TIMEOUT_MS * MIN_TIMEOUT_HEADROOM > TEST_TIMEOUT_MS) {
  throw new Error(
    `Test timeout misconfiguration: asyncUtilTimeout (${ASYNC_UTIL_TIMEOUT_MS}ms) needs at least ` +
      `${MIN_TIMEOUT_HEADROOM}x headroom under testTimeout (${TEST_TIMEOUT_MS}ms), or a wait that ` +
      `exhausts its budget will be killed before it can report why. See src/test/timeouts.ts.`
  );
}

/**
 * jsdom implements no `matchMedia`, and the app asks for it during render —
 * `useIsMobile` (hooks/use-mobile.ts) and the sidebar hook both call it, and the
 * theme reads `prefers-color-scheme`. Any component that renders a responsive
 * hook, however indirectly, dies with "window.matchMedia is not a function".
 *
 * This belongs here rather than in individual tests: it was being re-stubbed
 * ad hoc per file, so each new test that happened to pull in a responsive hook
 * failed until someone noticed and pasted the stub again. One stub, one place.
 *
 * Defaults to "no match", i.e. desktop width and light preference — the same
 * assumption jsdom's zero-sized layout already implies. A test that needs the
 * other branch should override this locally and say why.
 */
if (typeof window !== 'undefined' && !window.matchMedia) {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string): MediaQueryList =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addListener: () => {}, // deprecated, still called by some libs
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList,
  });
}

/**
 * jsdom implements no `Element.prototype.scrollIntoView`, and several components
 * call it from an effect — `MentionPopover` scrolls its selected row into view
 * on every render, `IngestPanel` brings the summoned paste box up, and
 * `ArtifactViewer`, `ChatTabStrip` and `ExtensionsView` all do the same.
 *
 * ⚠ **It is installed ONCE, for the whole process, and must never be removed
 * per test.** Two specs used to install it in `beforeEach` and DELETE it in
 * `afterEach`, and that is a race the polyfill cannot win, because the effect
 * that needs it does not run inside the test body:
 *
 *   - `scrollIntoView` is called from a PASSIVE effect. React commits a render
 *     in one scheduler callback and flushes that render's passive effects in
 *     the NEXT one, so a render committed by a late async resolution leaves
 *     `commitHookPassiveMountEffects` still queued when the test returns.
 *   - vitest runs `afterEach` hooks in reverse registration order, so a spec's
 *     own `afterEach` runs BEFORE this file's `cleanup()`. The property is
 *     therefore already gone when `cleanup()` unmounts — and unmounting is
 *     precisely what flushes the queued passive effects.
 *
 * The effect then throws `TypeError: … scrollIntoView is not a function` from
 * inside React, which unmounts the tree and fails a test that had already
 * asserted everything it came to assert. The window is a single scheduler turn
 * wide, so it opens on a loaded CI runner and almost never locally — i.e. it
 * reads as flakiness rather than as the deterministic ordering bug it is.
 * Measured with a probe that returns one macrotask after render: it throws
 * every time with the per-test install and never with this one.
 *
 * A spec that wants to ASSERT on the scroll should spy on the prototype with
 * `vi.spyOn(Element.prototype, 'scrollIntoView')` and restore the spy, which
 * puts this no-op back rather than leaving the property undefined. See
 * src/test/scrollIntoViewPolyfill.test.tsx for the regression guard.
 */
if (typeof Element !== 'undefined' && typeof Element.prototype.scrollIntoView !== 'function') {
  Object.defineProperty(Element.prototype, 'scrollIntoView', {
    configurable: true,
    writable: true,
    value: function scrollIntoView(): void {
      /* jsdom has no layout, so there is nothing to scroll. */
    },
  });
}

client.setConfig({
  baseUrl: 'http://localhost',
});
