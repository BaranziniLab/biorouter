import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  BOOT_SPLASH_GONE_EVENT,
  FIRST_RUN_NOTICE_HANDOFF_WAIT_MS,
  holdFirstRunNoticeForFirstPage,
  markFirstPageDrawn,
  resetFirstRunNoticeHandoffForTests,
  useFirstRunNoticeMayOpen,
} from './firstRunNoticeHandoff';

/**
 * FR-R5-1: the day-one notice opens when the launch has handed off, not when its count lands.
 *
 * Since FR-R4-4 the count starts at boot, so on a slower launch the answer is in before the boot
 * splash has finished fading and before `ProviderGuard` has stopped showing its empty loader. A
 * gate that opened the moment the answer was `show` drew the dialog under the fading wordmark and
 * over a bare ground (measured at 3x and 4x CPU). The hook under test is the one question the gate
 * asks before it draws: "may I open now?".
 *
 * What this file pins: nothing waits where the boot declared nothing; the two things a launch
 * hands off through are each enough to hold, and both must clear; the wait is bounded, because the
 * notice is a disclosure; and once the dialog may open, nothing takes that back.
 */

type Splash = { dismiss: () => void; isShown: () => boolean; isGone: () => boolean };

/** A splash whose state the test turns by hand, announcing its leaving the way the shipped one does. */
function installSplash() {
  let gone = false;
  const splash: Splash = { dismiss: () => undefined, isShown: () => true, isGone: () => gone };
  window.__brBootSplash = splash;
  return {
    leave() {
      gone = true;
      window.dispatchEvent(new Event(BOOT_SPLASH_GONE_EVENT));
    },
  };
}

beforeEach(() => {
  resetFirstRunNoticeHandoffForTests();
  delete window.__brBootSplash;
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete window.__brBootSplash;
});

describe('where nothing is waiting', () => {
  it('opens at once on a surface the boot did not reach', () => {
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(true);
  });

  it('never opens when there is nothing to open', () => {
    markFirstPageDrawn();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(false));
    expect(result.current).toBe(false);
  });

  it('ignores a splash controller that predates isGone', () => {
    window.__brBootSplash = { dismiss: () => undefined, isShown: () => true } as unknown as Splash;
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(true);
  });
});

describe('the first page', () => {
  it('holds from the boot declaration until the guard has drawn', () => {
    holdFirstRunNoticeForFirstPage();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(false);

    act(() => markFirstPageDrawn());
    expect(result.current).toBe(true);
  });

  it('holds nothing once the page has drawn, however late the declaration arrives', () => {
    markFirstPageDrawn();
    holdFirstRunNoticeForFirstPage();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(true);
  });

  it('is a one way latch: marking again changes nothing', () => {
    holdFirstRunNoticeForFirstPage();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    act(() => markFirstPageDrawn());
    act(() => markFirstPageDrawn());
    expect(result.current).toBe(true);
  });
});

describe('the boot splash', () => {
  it('holds while the splash is up and opens the moment it has left', () => {
    const splash = installSplash();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(false);

    act(() => splash.leave());
    expect(result.current).toBe(true);
  });

  it('opens at once when the splash left before the gate mounted', () => {
    const splash = installSplash();
    splash.leave();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(true);
  });

  it('waits for the splash even when the page has already drawn', () => {
    holdFirstRunNoticeForFirstPage();
    markFirstPageDrawn();
    const splash = installSplash();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(false);
    act(() => splash.leave());
    expect(result.current).toBe(true);
  });

  it('stops listening when the gate unmounts', () => {
    const remove = vi.spyOn(window, 'removeEventListener');
    installSplash();
    const { unmount } = renderHook(() => useFirstRunNoticeMayOpen(true));
    unmount();
    expect(remove).toHaveBeenCalledWith(BOOT_SPLASH_GONE_EVENT, expect.any(Function));
    remove.mockRestore();
  });
});

describe('both, in either order', () => {
  it('needs the splash gone AND the page drawn', () => {
    holdFirstRunNoticeForFirstPage();
    const splash = installSplash();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(false);

    act(() => splash.leave());
    expect(result.current).toBe(false); // the page has not drawn

    act(() => markFirstPageDrawn());
    expect(result.current).toBe(true);
  });

  it('opens the same way when the page draws first', () => {
    holdFirstRunNoticeForFirstPage();
    const splash = installSplash();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));

    act(() => markFirstPageDrawn());
    expect(result.current).toBe(false); // the splash is still up

    act(() => splash.leave());
    expect(result.current).toBe(true);
  });
});

describe('the bound', () => {
  it('opens anyway once the budget runs out, so a missed hold cannot hide the notice', () => {
    vi.useFakeTimers();
    holdFirstRunNoticeForFirstPage(); // nobody ever marks the page drawn
    installSplash(); // and the splash never leaves
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(false);

    act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_HANDOFF_WAIT_MS - 1));
    expect(result.current).toBe(false);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe(true);
  });

  it('counts from the moment there is a dialog to open, not from the launch', () => {
    vi.useFakeTimers();
    holdFirstRunNoticeForFirstPage();
    const { result, rerender } = renderHook(({ wanted }) => useFirstRunNoticeMayOpen(wanted), {
      initialProps: { wanted: false },
    });
    // The count has not answered: the budget has not begun.
    act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_HANDOFF_WAIT_MS * 2));
    rerender({ wanted: true });
    expect(result.current).toBe(false);

    act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_HANDOFF_WAIT_MS));
    expect(result.current).toBe(true);
  });

  it('takes a budget as an argument', () => {
    vi.useFakeTimers();
    holdFirstRunNoticeForFirstPage();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true, 50));
    act(() => vi.advanceTimersByTime(49));
    expect(result.current).toBe(false);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe(true);
  });
});

describe('once it may open', () => {
  it('stays allowed when a new launch is declared under an open dialog', () => {
    holdFirstRunNoticeForFirstPage();
    const { result } = renderHook(() => useFirstRunNoticeMayOpen(true));
    act(() => markFirstPageDrawn());
    expect(result.current).toBe(true);

    // The dialog is up. A reset and a fresh declaration would, without the latch, close it.
    act(() => resetFirstRunNoticeHandoffForTests());
    act(() => holdFirstRunNoticeForFirstPage());
    expect(result.current).toBe(true);
  });

  it('stays allowed when a hold is declared after the dialog was allowed', () => {
    const { result, rerender } = renderHook(() => useFirstRunNoticeMayOpen(true));
    expect(result.current).toBe(true);
    act(() => {
      installSplash();
      holdFirstRunNoticeForFirstPage();
    });
    rerender();
    expect(result.current).toBe(true);
  });
});

describe('the boot and the guard', () => {
  // The two assertions a render cannot make. Every behaviour above would keep passing if the boot
  // stopped declaring the hold or the guard stopped marking the page, and the dialog would be back
  // under the fading wordmark: the hold exists only because the boot takes it, and it clears only
  // because the guard says so.
  const read = async (relative: string) => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    // `process.cwd()` is `ui/desktop`, where vitest.config lives.
    return readFileSync(resolve(process.cwd(), relative), 'utf8')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^[ \t]*\/\/.*$/gm, '');
  };

  it('is declared by the boot, once the daemon client is configured and before React renders', async () => {
    const code = await read('src/renderer.tsx');
    // Anti-vacuity: this is the boot function, not a stub left behind.
    expect(code).toContain('client.setConfig(daemonClientConfig(');
    expect(code).toContain("from './components/privacy/firstRunNoticeHandoff'");

    const configured = code.indexOf('client.setConfig(daemonClientConfig(');
    const declared = code.indexOf('holdFirstRunNoticeForFirstPage();');
    const rendered = code.indexOf('ReactDOM.createRoot(');
    expect(declared).toBeGreaterThan(configured);
    expect(declared).toBeLessThan(rendered);
    // Inside the `!isLauncher` branch, once: the launcher mounts no guard and no gate.
    expect(code.slice(code.indexOf('if (!isLauncher) {'), rendered)).toContain(
      'holdFirstRunNoticeForFirstPage();'
    );
    expect(code.split('holdFirstRunNoticeForFirstPage();').length - 1).toBe(1);
  });

  it('is cleared by the guard when it stops showing its loader, and by nothing else', async () => {
    const guard = await read('src/components/ProviderGuard.tsx');
    expect(guard).toContain("from './privacy/firstRunNoticeHandoff'");
    expect(guard).toMatch(/const firstPageDrawn = passesThrough \|\| !isChecking;/);
    expect(guard).toMatch(/if \(firstPageDrawn\) markFirstPageDrawn\(\);/);

    // Every non-test source file that calls it: the definition and the guard, nothing else.
    const { readdirSync, readFileSync } = await import('node:fs');
    const { join, relative, resolve } = await import('node:path');
    const root = resolve(process.cwd(), 'src');
    const callers: string[] = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) walk(path);
        else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
          if (readFileSync(path, 'utf8').includes('markFirstPageDrawn(')) {
            callers.push(relative(process.cwd(), path));
          }
        }
      }
    };
    walk(root);
    expect(callers.sort()).toEqual([
      'src/components/ProviderGuard.tsx',
      'src/components/privacy/firstRunNoticeHandoff.ts',
    ]);
  });

  it('keeps the entry chunk free of anything but React', async () => {
    const source = await read('src/components/privacy/firstRunNoticeHandoff.ts');
    const imports = [...source.matchAll(/^import .* from '([^']+)';$/gm)].map((m) => m[1]);
    expect(imports).toEqual(['react']);
  });
});
