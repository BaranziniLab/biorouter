import { describe, expect, it, vi } from 'vitest';
import {
  TITLE_BAR_SLACK_PX,
  WINDOW_FRAME_SLACK_PX,
  createViewportPinReporter,
  describeViewportPin,
  installViewportPinWarning,
  type ViewportMetrics,
} from './viewportPin';

/**
 * The decision this file guards is the one thing about the viewport warning
 * that can be wrong in a way nobody notices: it fires on tooling state, in
 * development only, so a false alarm lands squarely in front of a developer who
 * is already hunting a layout bug, and a miss puts them back into the CSS for an
 * hour. Both failure modes are cheap to assert here and expensive to discover
 * in a running app, which is exactly why the arithmetic lives in a module with
 * no React and no DOM.
 *
 * The sizes below are the ones actually measured on 2026-09-08 (see the module
 * header), not invented ones.
 */

const mac = (over: Partial<ViewportMetrics> = {}): ViewportMetrics => ({
  innerWidth: 1440,
  innerHeight: 900,
  outerWidth: 1440,
  outerHeight: 928,
  platform: 'darwin',
  ...over,
});

describe('describeViewportPin', () => {
  it('reports the real 2026-09-08 pin: inner 1440×900 inside a 1638×963 window', () => {
    const message = describeViewportPin(mac({ outerWidth: 1638, outerHeight: 963 }));
    expect(message).not.toBeNull();
    expect(message).toContain('1440×900');
    expect(message).toContain('1638×963');
  });

  it('reports the second instance too, where only the height differed (1440×1000)', () => {
    // Width matched exactly, so a width-only check would have missed this one.
    const message = describeViewportPin(mac({ outerWidth: 1440, outerHeight: 1000 }));
    expect(message).not.toBeNull();
    expect(message).toContain('1440×900');
    expect(message).toContain('1440×1000');
  });

  it('names both sizes, the check command and the restart cure', () => {
    const message = describeViewportPin(mac({ outerWidth: 1638, outerHeight: 963 }))!;
    expect(message).toContain('1440×900');
    expect(message).toContain('1638×963');
    expect(message).toContain('npm run cdp:viewport-check');
    expect(message).toContain('restart');
    // The line exists to stop someone editing CSS, so it must say so outright.
    expect(message).toContain('NOT a layout bug');
  });

  it('stays silent when the viewport matches the window exactly', () => {
    expect(describeViewportPin(mac({ innerHeight: 928, outerHeight: 928 }))).toBeNull();
  });

  it('stays silent for a macOS title bar alone', () => {
    // 28px of title bar, no side frame — the ordinary unpinned macOS reading.
    expect(describeViewportPin(mac({ outerHeight: 928 }))).toBeNull();
    // The boundary is inclusive: exactly the slack is still not a pin.
    expect(describeViewportPin(mac({ outerHeight: 900 + TITLE_BAR_SLACK_PX }))).toBeNull();
    expect(describeViewportPin(mac({ outerHeight: 900 + TITLE_BAR_SLACK_PX + 1 }))).not.toBeNull();
  });

  it('treats ANY width difference on macOS as a pin, because there is no side frame', () => {
    expect(describeViewportPin(mac({ outerWidth: 1441 }))).not.toBeNull();
    expect(describeViewportPin(mac({ outerWidth: 1448 }))).not.toBeNull();
  });

  it('allows a side frame on Windows and Linux', () => {
    for (const platform of ['win32', 'linux']) {
      const framed = {
        platform,
        outerWidth: 1440 + WINDOW_FRAME_SLACK_PX,
        outerHeight: 900 + TITLE_BAR_SLACK_PX + WINDOW_FRAME_SLACK_PX,
      };
      expect(describeViewportPin(mac(framed))).toBeNull();
      // One pixel past the frame allowance is a pin on those platforms too.
      expect(
        describeViewportPin(mac({ ...framed, outerWidth: 1440 + WINDOW_FRAME_SLACK_PX + 1 }))
      ).not.toBeNull();
      expect(
        describeViewportPin(
          mac({ ...framed, outerHeight: 900 + TITLE_BAR_SLACK_PX + WINDOW_FRAME_SLACK_PX + 1 })
        )
      ).not.toBeNull();
    }
  });

  it('treats a viewport LARGER than its window as a pin on every platform', () => {
    // Window chrome only ever makes `outer` bigger, so this direction can only
    // come from emulation — and the signed comparison is what catches it inside
    // the Windows frame allowance, where an absolute difference would not.
    expect(describeViewportPin(mac({ innerWidth: 1450 }))).not.toBeNull();
    expect(
      describeViewportPin(mac({ platform: 'win32', innerWidth: 1450, outerWidth: 1440 }))
    ).not.toBeNull();
    expect(
      describeViewportPin(mac({ platform: 'win32', innerHeight: 940, outerHeight: 928 }))
    ).not.toBeNull();
  });

  it('says nothing when a dimension is unmeasurable', () => {
    // A minimized or not-yet-shown window reports 0 for `outer`; reading that as
    // "the viewport is larger than the window" would warn on a state that is not
    // a pin at all.
    expect(describeViewportPin(mac({ outerWidth: 0, outerHeight: 0 }))).toBeNull();
    expect(describeViewportPin(mac({ innerWidth: 0 }))).toBeNull();
    expect(describeViewportPin(mac({ outerHeight: Number.NaN }))).toBeNull();
    expect(describeViewportPin(mac({ innerHeight: Number.POSITIVE_INFINITY }))).toBeNull();
    expect(describeViewportPin(mac({ outerWidth: -1638 }))).toBeNull();
  });

  it('treats an unknown platform as framed rather than as macOS', () => {
    // Guessing "no side frame" for a platform we have never measured would turn
    // every window on it into a false alarm.
    expect(
      describeViewportPin(mac({ platform: 'freebsd', outerWidth: 1440 + WINDOW_FRAME_SLACK_PX }))
    ).toBeNull();
  });
});

describe('createViewportPinReporter', () => {
  it('warns once per distinct pinned size, not once per resize event', () => {
    const warn = vi.fn();
    const report = createViewportPinReporter(warn);
    const pinned = mac({ outerWidth: 1638, outerHeight: 963 });

    report(pinned);
    report(pinned);
    report(pinned);
    expect(warn).toHaveBeenCalledTimes(1);

    // A different window size around the same pinned viewport is a new reading.
    report(mac({ outerWidth: 1900, outerHeight: 1050 }));
    expect(warn).toHaveBeenCalledTimes(2);

    // ...and a different pinned viewport is a second, separate mistake.
    report(mac({ innerWidth: 1280, innerHeight: 800, outerWidth: 1900, outerHeight: 1050 }));
    expect(warn).toHaveBeenCalledTimes(3);
  });

  it('never warns for an unpinned window, however many times it is asked', () => {
    const warn = vi.fn();
    const report = createViewportPinReporter(warn);
    for (let i = 0; i < 50; i += 1) report(mac());
    expect(warn).not.toHaveBeenCalled();
  });
});

describe('installViewportPinWarning', () => {
  function fakeWindow(metrics: Omit<ViewportMetrics, 'platform'>) {
    const listeners = new Set<() => void>();
    return {
      ...metrics,
      addEventListener: (type: string, fn: () => void) => {
        if (type === 'resize') listeners.add(fn);
      },
      removeEventListener: (type: string, fn: () => void) => {
        if (type === 'resize') listeners.delete(fn);
      },
      fireResize: () => listeners.forEach((fn) => fn()),
      listenerCount: () => listeners.size,
    };
  }

  it('checks on install and again after a debounced resize', () => {
    vi.useFakeTimers();
    const warn = vi.fn();
    const win = fakeWindow({
      innerWidth: 1440,
      innerHeight: 900,
      outerWidth: 1440,
      outerHeight: 928,
    });

    const teardown = installViewportPinWarning({
      win: win as unknown as Window,
      warn,
      platform: 'darwin',
      debounceMs: 400,
    });
    expect(warn).not.toHaveBeenCalled();

    // The window grows; the emulated viewport does not follow.
    win.outerWidth = 1638;
    win.outerHeight = 963;
    win.fireResize();
    win.fireResize();
    win.fireResize();
    expect(warn).not.toHaveBeenCalled(); // still inside the debounce window
    vi.advanceTimersByTime(400);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toContain('1638×963');

    // Further resizes at the same reading stay quiet.
    win.fireResize();
    vi.advanceTimersByTime(400);
    expect(warn).toHaveBeenCalledTimes(1);

    teardown();
    expect(win.listenerCount()).toBe(0);
    vi.useRealTimers();
  });

  it('defaults to the real window, and its teardown detaches from it', () => {
    // jsdom reports inner === outer, i.e. an unpinned window, so this asserts the
    // wiring itself: it attaches to the ambient window with no options at all and
    // removes exactly what it added.
    const warn = vi.fn();
    const added = vi.spyOn(window, 'addEventListener');
    const removed = vi.spyOn(window, 'removeEventListener');

    const teardown = installViewportPinWarning({ warn });
    expect(added).toHaveBeenCalledWith('resize', expect.any(Function));
    expect(warn).not.toHaveBeenCalled();

    teardown();
    expect(removed).toHaveBeenCalledWith('resize', added.mock.calls[0][1]);

    added.mockRestore();
    removed.mockRestore();
  });
});
