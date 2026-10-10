import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  DEFAULT_GHOST_SPEC,
  DEFAULT_GHOST_STYLE,
  DragGhostWindowController,
  GHOST_MAX_WIDTH,
  GHOST_OPAQUE_INSET,
  GHOST_PROBE_SCRIPT,
  GHOST_TRANSPARENT_INSET,
  escapeHtml,
  ghostWindowBounds,
  ghostWindowDataUrl,
  ghostWindowHtml,
  sanitizeColor,
  sanitizeCssValue,
  sanitizeGhostSpec,
  type DragGhostHost,
  type GhostSpec,
  type GhostWindowHandle,
} from './dragGhostWindow';
import type { Rect } from './windowDrag';

/**
 * The torn-off tab's cross-desktop ghost — issue #75, design Phase 4b.
 *
 * ⚠ WHAT THIS FILE DOES NOT TEST, AND CANNOT.
 *
 * The feature is "a ghost that follows the cursor onto the desktop". Nothing
 * here asserts that, because nothing in this repo can: the suite runs in jsdom,
 * which has no layout, no pointer capture, no screen and no second window, and
 * `document.elementFromPoint` is missing outright rather than returning null.
 * `useTabDragReorder.crossWindow.test.tsx` and the shell suites state the same
 * limitation in their own headers.
 *
 * Three properties therefore have NO automated cover at all and need real OS
 * input on a real desktop to confirm:
 *
 *   1. that the ghost window does not steal focus. This is the one that ends the
 *      gesture if it is wrong — the source window holds the pointer capture, and
 *      raising or focusing anything drops it. The recipe (`focusable: false`,
 *      `setIgnoreMouseEvents(true)`, `showInactive()`) lives in `main.ts`, so it
 *      is asserted by READING that file — see the last describe block. Reading
 *      is not running: it cannot prove macOS honours the flags. It can prove
 *      nobody deleted them, which is the failure that actually happens.
 *   2. that the window paints outside the source window's frame.
 *   3. that per-monitor DPI lands it on the right display. The DIP conversion is
 *      the identity on macOS, so even a manual check on this machine cannot see
 *      that class of bug (`windowDrag.ts` D4).
 *
 * What IS covered below is everything that survives being separated from the OS:
 * the placement arithmetic, the sanitising of what the renderer said, and the
 * lifecycle state machine — one window per drag, and destroyed on every exit,
 * including an exit that happens while the window is still being created.
 */

const SPEC: GhostSpec = {
  title: 'Volcano plot',
  width: 140,
  height: 30,
  grabOffset: { x: 20, y: 15 },
  style: DEFAULT_GHOST_STYLE,
};

describe('ghostWindowBounds', () => {
  it('puts the ghost under the cursor at the same offset the tab was grabbed by', () => {
    // Deliberately the same expression `tornOffWindowBounds` uses for the window
    // this ghost becomes, so the two land at one origin rather than by agreement.
    expect(ghostWindowBounds({ x: 1000, y: 500 }, SPEC, 0)).toEqual({
      x: 980,
      y: 485,
      width: 140,
      height: 30,
    });
  });

  it('grows the window by the transparent inset on every side and shifts the origin to match', () => {
    const bounds = ghostWindowBounds({ x: 1000, y: 500 }, SPEC, GHOST_TRANSPARENT_INSET);
    expect(bounds).toEqual({
      x: 980 - GHOST_TRANSPARENT_INSET,
      y: 485 - GHOST_TRANSPARENT_INSET,
      width: 140 + GHOST_TRANSPARENT_INSET * 2,
      height: 30 + GHOST_TRANSPARENT_INSET * 2,
    });
    // The chip itself is still at the same screen point: the inset is padding,
    // not a move.
    expect(bounds.x + GHOST_TRANSPARENT_INSET).toBe(980);
    expect(bounds.y + GHOST_TRANSPARENT_INSET).toBe(485);
  });

  it('is NOT clamped to any work area — hanging off the display is the feature', () => {
    // The whole point of #75 is that the ghost goes where the cursor goes. A
    // clamp here would reintroduce the reported bug one layer down.
    expect(ghostWindowBounds({ x: -300, y: -200 }, SPEC, 0)).toEqual({
      x: -320,
      y: -215,
      width: 140,
      height: 30,
    });
  });

  it('rounds to whole pixels, because BrowserWindow bounds are integers', () => {
    const bounds = ghostWindowBounds({ x: 100.6, y: 50.4 }, SPEC, 0);
    expect(Number.isInteger(bounds.x)).toBe(true);
    expect(Number.isInteger(bounds.y)).toBe(true);
    expect(bounds).toMatchObject({ x: 81, y: 35 });
  });
});

describe('sanitizeCssValue', () => {
  it('passes ordinary resolved values through', () => {
    expect(sanitizeCssValue('rgb(24, 22, 20)', 'x')).toBe('rgb(24, 22, 20)');
    expect(sanitizeCssValue('0px 8px 24px 0px rgba(0, 0, 0, 0.4)', 'x')).toBe(
      '0px 8px 24px 0px rgba(0, 0, 0, 0.4)'
    );
    expect(sanitizeCssValue('"Inter", system-ui, sans-serif', 'x')).toBe(
      '"Inter", system-ui, sans-serif'
    );
  });

  it('collapses the newlines a multi-line token (--shadow-popover) comes back with', () => {
    expect(sanitizeCssValue('0 1px 2px #000,\n    0 8px 24px #000', 'x')).toBe(
      '0 1px 2px #000, 0 8px 24px #000'
    );
  });

  it('refuses anything that could close the declaration and open another', () => {
    // These are spliced into a <style> block in a DIFFERENT window, so a value
    // that escapes its rule is that window's stylesheet rewritten by the page
    // that supplied it.
    expect(sanitizeCssValue('red; } body { background: url(http://x) ', 'fb')).toBe('fb');
    expect(sanitizeCssValue('</style><script>alert(1)</script>', 'fb')).toBe('fb');
    expect(sanitizeCssValue('url(http://evil/x.png)', 'fb')).toBe('fb');
    expect(sanitizeCssValue('@import "http://evil"', 'fb')).toBe('fb');
  });

  it('refuses an unbalanced paren or quote even though every character is allowed', () => {
    expect(sanitizeCssValue('rgb(1, 2, 3', 'fb')).toBe('fb');
    expect(sanitizeCssValue('"Inter', 'fb')).toBe('fb');
  });

  it('refuses a var() reference, which names nothing in the ghost window', () => {
    expect(sanitizeCssValue('var(--color-coral-500)', 'fb')).toBe('fb');
  });

  it('falls back for a non-string, an empty string, and an essay', () => {
    expect(sanitizeCssValue(undefined, 'fb')).toBe('fb');
    expect(sanitizeCssValue(42, 'fb')).toBe('fb');
    expect(sanitizeCssValue('   ', 'fb')).toBe('fb');
    expect(sanitizeCssValue('a'.repeat(201), 'fb')).toBe('fb');
  });
});

describe('sanitizeColor', () => {
  it('accepts the shapes a computed colour actually arrives in', () => {
    for (const value of ['#fff', '#cf6d47', 'rgb(1, 2, 3)', 'rgba(1, 2, 3, 0.5)', 'transparent']) {
      expect(sanitizeColor(value, 'fb')).toBe(value);
    }
    expect(sanitizeColor('oklch(0.7 0.1 40)', 'fb')).toBe('oklch(0.7 0.1 40)');
  });

  it('rejects a value that is not a colour at all', () => {
    // The one that matters: if `getPropertyValue('--accent-bar')` ever returned
    // the SPECIFIED text instead of the substituted one, inlining it would name a
    // custom property the ghost window has never heard of and the outline would
    // silently vanish.
    expect(sanitizeColor('var(--color-coral-500)', '#cf6d47')).toBe('#cf6d47');
    expect(sanitizeColor('12px', '#cf6d47')).toBe('#cf6d47');
    expect(sanitizeColor('', '#cf6d47')).toBe('#cf6d47');
  });
});

describe('sanitizeGhostSpec', () => {
  it('reads a well-formed probe result', () => {
    const spec = sanitizeGhostSpec({
      title: 'Volcano plot',
      width: 140,
      height: 30,
      grabOffsetX: 20,
      grabOffsetY: 15,
      style: { background: 'rgb(255, 255, 255)', color: 'rgb(42, 37, 32)', accent: '#cf6d47' },
    });
    expect(spec.title).toBe('Volcano plot');
    expect(spec.width).toBe(140);
    expect(spec.grabOffset).toEqual({ x: 20, y: 15 });
    expect(spec.style.background).toBe('rgb(255, 255, 255)');
    // Unsupplied fields take the default rather than becoming `undefined` in a
    // CSS declaration, which would invalidate the whole rule.
    expect(spec.style.radius).toBe(DEFAULT_GHOST_STYLE.radius);
  });

  it('falls back wholesale when the probe found no ghost', () => {
    expect(sanitizeGhostSpec(null)).toEqual(DEFAULT_GHOST_SPEC);
  });

  it('clamps a measurement to .br-tab’s own width band', () => {
    expect(sanitizeGhostSpec({ width: 4000, height: 30 }).width).toBe(GHOST_MAX_WIDTH);
    expect(sanitizeGhostSpec({ width: 1, height: 30 }).width).toBe(88);
  });

  it('replaces NaN and Infinity rather than propagating them into a window rect', () => {
    // `Math.round(NaN - 0)` is NaN, and a BrowserWindow at NaN,NaN is the exact
    // failure the wire-shape bug produced for torn-off windows (windowDrag.ts).
    const spec = sanitizeGhostSpec({
      width: Number.NaN,
      height: Number.POSITIVE_INFINITY,
      grabOffsetX: Number.NaN,
      grabOffsetY: Number.NaN,
    });
    const bounds = ghostWindowBounds({ x: 10, y: 10 }, spec, 0);
    for (const value of Object.values(bounds)) expect(Number.isFinite(value)).toBe(true);
  });

  it('keeps the grab offset inside the chip', () => {
    const spec = sanitizeGhostSpec({ width: 100, height: 30, grabOffsetX: 900, grabOffsetY: -5 });
    expect(spec.grabOffset.x).toBeLessThanOrEqual(100);
    expect(spec.grabOffset.y).toBeGreaterThanOrEqual(0);
  });

  it('strips control characters and caps the title', () => {
    const spec = sanitizeGhostSpec({ title: `a\u0000b\u001fc${'x'.repeat(400)}` });
    expect(spec.title.startsWith('a b c')).toBe(true);
    expect(spec.title).toHaveLength(160);
  });
});

describe('ghostWindowHtml', () => {
  it('escapes the title — a chat can be named anything', () => {
    const html = ghostWindowHtml(
      { ...SPEC, title: '<img src=x onerror=alert(1)>' },
      {
        transparent: true,
      }
    );
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img');
  });

  it('draws the detached look: flat, dashed accent outline, shadow', () => {
    const html = ghostWindowHtml(SPEC, { transparent: true });
    expect(html).toContain('outline: 2px dashed color-mix(in srgb, #cf6d47 55%, transparent)');
    expect(html).toContain(`box-shadow: ${DEFAULT_GHOST_STYLE.shadow}`);
    // No tilt: `.br-tab-ghost[data-detach='true']` returns rotation to 0 because
    // a flat outlined rectangle reads as a window and a tilted one as a tab.
    expect(html).not.toContain('rotate(');
  });

  it('moves the dashed edge inside the box and drops the shadow where the window is opaque', () => {
    const html = ghostWindowHtml(SPEC, { transparent: false });
    expect(html).toContain('border: 2px dashed');
    expect(html).not.toContain('outline:');
    expect(html).not.toContain('box-shadow');
    // The window IS the chip off darwin, so the page ground must be the chip's,
    // not the transparent nothing a compositing platform would show.
    expect(html).toContain(`background: ${DEFAULT_GHOST_STYLE.background}`);
  });

  it('carries its own CSP, since a data: URL gets no headers from anywhere', () => {
    expect(ghostWindowHtml(SPEC, { transparent: true })).toContain(
      "default-src 'none'; style-src 'unsafe-inline'"
    );
  });

  it('round-trips through the data URL', () => {
    const html = ghostWindowHtml(SPEC, { transparent: true });
    const url = ghostWindowDataUrl(html);
    expect(url.startsWith('data:text/html;charset=utf-8,')).toBe(true);
    expect(decodeURIComponent(url.slice('data:text/html;charset=utf-8,'.length))).toBe(html);
  });
});

describe('escapeHtml', () => {
  it('covers the five characters that matter in an attribute or a text node', () => {
    expect(escapeHtml(`&<>"'`)).toBe('&amp;&lt;&gt;&quot;&#39;');
  });
});

describe('GHOST_PROBE_SCRIPT', () => {
  it('reads the element ChatDropOverlay actually renders', () => {
    // Main has no other way to learn the tab's title, size, grab offset or theme
    // colours: `tab-drag:move` carries `{screenX, screenY}` and nothing else.
    // This pins the coupling so a rename of the class shows up here.
    expect(GHOST_PROBE_SCRIPT).toContain('.br-tab-ghost');
    expect(GHOST_PROBE_SCRIPT).toContain('data-grab-x');
    expect(GHOST_PROBE_SCRIPT).toContain('data-grab-y');
    expect(GHOST_PROBE_SCRIPT).toContain('--accent-bar');
  });
});

// ── The lifecycle ──────────────────────────────────────────────────────────

interface FakeWindow extends GhostWindowHandle {
  positions: Array<{ x: number; y: number }>;
  shown: number;
  destroyed: number;
}

function fakeWindow(): FakeWindow {
  const win: FakeWindow = {
    positions: [],
    shown: 0,
    destroyed: 0,
    setPosition: (x, y) => void win.positions.push({ x, y }),
    show: () => void win.shown++,
    destroy: () => void win.destroyed++,
  };
  return win;
}

/**
 * A host whose `createWindow` can be PARKED by the test, because every bug worth
 * catching in this controller lives in the gap between asking for a window and
 * getting one — that is where a drag can end under a window that is still being
 * born.
 */
function fakeHost(options?: { probe?: unknown }) {
  const created: Array<{ bounds: Rect; spec: GhostSpec; win: FakeWindow }> = [];
  const notified: Array<{ windowId: number; active: boolean }> = [];
  const parked: Array<() => void> = [];
  let parking = false;

  const host: DragGhostHost = {
    probeSource: async () => options?.probe ?? null,
    createWindow: async (_id, bounds, spec) => {
      if (parking) await new Promise<void>((resolve) => parked.push(resolve));
      const win = fakeWindow();
      created.push({ bounds, spec, win });
      return win;
    },
    notifySource: (windowId, active) => void notified.push({ windowId, active }),
    onError: () => {},
  };

  return {
    host,
    created,
    notified,
    /** Every `createWindow` from here on waits. The returned call releases all of them. */
    parkCreate() {
      parking = true;
      return () => {
        for (const resolve of parked.splice(0)) resolve();
      };
    },
  };
}

/**
 * Drain the microtask queue.
 *
 * A macrotask rather than a fixed number of `Promise.resolve()`s: the create
 * path is two awaits deep and each `async` hop costs an unspecified number of
 * ticks, so counting them is a test that passes until someone adds an await.
 */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/** `Array.prototype.at` is past this tsconfig's lib target. */
function last<T>(items: T[]): T | undefined {
  return items[items.length - 1];
}

describe('DragGhostWindowController', () => {
  it('creates exactly one window however many moves arrive', async () => {
    const fake = fakeHost();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });

    controller.follow(7, { x: 100, y: 100 });
    controller.follow(7, { x: 101, y: 100 });
    controller.follow(7, { x: 102, y: 100 });
    await flush();
    controller.follow(7, { x: 103, y: 100 });

    expect(fake.created).toHaveLength(1);
    expect(controller.isShowing).toBe(true);
    expect(fake.created[0].win.shown).toBe(1);
  });

  it('repositions on every later move instead of rebuilding', async () => {
    const fake = fakeHost();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    await flush();
    controller.follow(7, { x: 140, y: 160 });

    const win = fake.created[0].win;
    // The default spec's grab offset is half the chip: 80 x 16.
    expect(last(win.positions)).toEqual({
      x: 140 - DEFAULT_GHOST_SPEC.grabOffset.x,
      y: 160 - DEFAULT_GHOST_SPEC.grabOffset.y,
    });
    expect(fake.created).toHaveLength(1);
  });

  it('shows the window at the LATEST point, not the one it was created for', async () => {
    // Creation is two awaits long. A ghost that appeared where the cursor was
    // when the drag left the window would jump across the screen on its first
    // frame.
    const fake = fakeHost();
    const resume = fake.parkCreate();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });

    controller.follow(7, { x: 100, y: 100 });
    await flush();
    controller.follow(7, { x: 400, y: 300 });
    resume();
    await flush();

    const win = fake.created[0].win;
    expect(last(win.positions)).toEqual({
      x: 400 - DEFAULT_GHOST_SPEC.grabOffset.x,
      y: 300 - DEFAULT_GHOST_SPEC.grabOffset.y,
    });
    expect(win.shown).toBe(1);
  });

  it('tells the source to hide its DOM ghost only once the OS one is up, and to restore it after', async () => {
    const fake = fakeHost();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    expect(fake.notified).toHaveLength(0); // nothing on screen yet — do not blind the user
    await flush();
    expect(fake.notified).toEqual([{ windowId: 7, active: true }]);

    controller.release('drag ended');
    expect(last(fake.notified)).toEqual({ windowId: 7, active: false });
  });

  it('destroys the window on release', async () => {
    const fake = fakeHost();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    await flush();
    controller.release('drag ended');

    expect(fake.created[0].win.destroyed).toBe(1);
    expect(controller.isShowing).toBe(false);
    expect(controller.sourceWindow).toBeNull();
  });

  it('destroys a window that is STILL BEING BORN when the drag ends', async () => {
    // The gap between "the user released" and "the window finished loading" is
    // the only way to strand a click-through, always-on-top chip on the desktop
    // with no gesture behind it and no way to dismiss it.
    const fake = fakeHost();
    const resume = fake.parkCreate();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });

    controller.follow(7, { x: 100, y: 100 });
    await flush();
    controller.release('drag committed');
    resume();
    await flush();

    expect(fake.created).toHaveLength(1);
    expect(fake.created[0].win.destroyed).toBe(1);
    expect(fake.created[0].win.shown).toBe(0);
    expect(controller.isShowing).toBe(false);
    // And the renderer is told to bring its own ghost back even though ours was
    // never shown, so a cancelled drag can never leave the tab invisible.
    expect(fake.notified).toEqual([{ windowId: 7, active: false }]);
  });

  it('does not re-arm a stale create: a move after the release starts a fresh one', async () => {
    const fake = fakeHost();
    const resume = fake.parkCreate();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });

    controller.follow(7, { x: 100, y: 100 });
    await flush();
    controller.release('drag ended');
    // The abandoned create must not leave the controller thinking one is still
    // coming, or the next time the cursor leaves the window nothing appears.
    expect(controller.isPending).toBe(false);

    controller.follow(7, { x: 200, y: 200 });
    await flush();
    resume();
    await flush();

    expect(fake.created).toHaveLength(2);
    expect(fake.created[0].win.destroyed).toBe(1); // the abandoned one
    expect(fake.created[1].win.shown).toBe(1); // the live one
    expect(controller.isShowing).toBe(true);
  });

  it('releases only for the window that owns the ghost', async () => {
    const fake = fakeHost();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    await flush();

    controller.releaseIfSource(9, 'window closed');
    expect(controller.isShowing).toBe(true);

    controller.releaseIfSource(7, 'window closed');
    expect(controller.isShowing).toBe(false);
    expect(fake.created[0].win.destroyed).toBe(1);
  });

  it('hands the ghost over when a second window starts dragging', async () => {
    // The first window's `tab-drag:end` can be lost (a crash, a reload). Two
    // ghosts on the desktop would be worse than one late handover.
    const fake = fakeHost();
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    await flush();
    controller.follow(9, { x: 300, y: 300 });
    await flush();

    expect(fake.created[0].win.destroyed).toBe(1);
    expect(controller.sourceWindow).toBe(9);
    expect(fake.created).toHaveLength(2);
    expect(fake.created[1].win.destroyed).toBe(0);
  });

  it('survives a host that refuses to build the window', async () => {
    const host: DragGhostHost = {
      probeSource: async () => null,
      createWindow: async () => null,
      notifySource: vi.fn(),
      onError: vi.fn(),
    };
    const controller = new DragGhostWindowController(host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    await flush();

    expect(controller.isShowing).toBe(false);
    expect(controller.isPending).toBe(false);
    // NOT told to hide: with no OS ghost, the clamped DOM ghost is the only one
    // there is and it must stay on screen.
    expect(host.notifySource).not.toHaveBeenCalledWith(7, true);
  });

  it('survives a probe that throws, leaving the clamped DOM ghost as the only one', async () => {
    const onError = vi.fn();
    const created: Rect[] = [];
    const host: DragGhostHost = {
      probeSource: async () => {
        throw new Error('renderer went away');
      },
      createWindow: async (_id, bounds) => {
        created.push(bounds);
        return fakeWindow();
      },
      notifySource: () => {},
      onError,
    };
    const controller = new DragGhostWindowController(host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 100, y: 100 });
    await flush();

    expect(onError).toHaveBeenCalled();
    expect(created).toHaveLength(0);
    expect(controller.isPending).toBe(false);
    // Recoverable: the next move tries again rather than the drag being stuck
    // with no ghost for good.
    expect(controller.isShowing).toBe(false);
  });

  it('uses the probed measurements, not the defaults, once it has them', async () => {
    const fake = fakeHost({
      probe: {
        title: 'Cohort QC',
        width: 150,
        height: 28,
        grabOffsetX: 12,
        grabOffsetY: 9,
        style: { background: 'rgb(20, 20, 20)', color: 'rgb(240, 240, 240)', accent: '#16a0ac' },
      },
    });
    const controller = new DragGhostWindowController(fake.host, { inset: GHOST_OPAQUE_INSET });
    controller.follow(7, { x: 500, y: 400 });
    await flush();

    expect(fake.created[0].bounds).toEqual({ x: 488, y: 391, width: 150, height: 28 });
    expect(fake.created[0].spec.title).toBe('Cohort QC');
    // The Alma Mater teal, carried straight through — a hardcoded accent would
    // paint every theme family in Parchment coral.
    expect(ghostWindowHtml(fake.created[0].spec, { transparent: true })).toContain('#16a0ac');
  });
});

/**
 * ⚠ **The half of this feature that lives in `main.ts`, where no test runs.**
 *
 * Everything above talks to a fake host. The real host is `main.ts`: the
 * `BrowserWindow` recipe and the four places the controller is driven from. That
 * file imports Electron at the top, so it cannot be imported here at all, and
 * `ui/desktop` has no main-process test of any kind. The result was that the
 * three flags issue #75 calls **mandatory** — `focusable: false`,
 * `setIgnoreMouseEvents(true)`, and no `vibrancy` — had nothing asserting them:
 * deleting any one of them left the whole suite green and broke the gesture on
 * the desktop, where the only symptom is "the drag stops working sometimes".
 *
 * So these read the shipped source, the way `workspaceChannelCsp.test.ts` and
 * `styles/measures.test.ts` do for facts only the source can settle.
 *
 * **What this is not.** It cannot show that macOS honours `focusable: false`,
 * that the window paints over another app, or that the pointer capture survives.
 * Those need real OS input and are listed at the top of this file as uncovered.
 * The failure this catches is the ordinary one: a later edit tidying an option
 * away, or a `showInactive()` "simplified" to `show()`.
 */
describe('the main-process half, read from main.ts', () => {
  const source = readFileSync(join(__dirname, 'main.ts'), 'utf8');

  /** The text between `marker` and the first line that closes it at `end`. */
  const block = (marker: string, end: string) => {
    const start = source.indexOf(marker);
    expect(start, `${marker} missing from main.ts`).toBeGreaterThan(-1);
    const stop = source.indexOf(end, start);
    expect(stop, `${marker} is never closed by ${JSON.stringify(end)}`).toBeGreaterThan(start);
    return source.slice(start, stop);
  };

  /**
   * ⚠ **Comments out, before any assertion about what the CODE does.** Both
   * halves of this file explain themselves at length, and every explanation
   * names the thing it is explaining: the recipe's comment says "NO PRELOAD",
   * the commit handler's says "returning early would leave it there for good".
   * A guard reading the raw slice therefore passes on `preload` and finds a
   * `return` 340 characters before the real one — both measured, both on
   * correct code. That is now the fourth guard in this repo to match its own
   * prose, so the stripping is the default here rather than a patch.
   */
  const code = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '');

  const recipe = () => block('async function createDragGhostWindow(', '\n}');

  /**
   * ⚠ Each of these is one line in `main.ts` and each is load-bearing on its
   * own. `focusable: false` and `setIgnoreMouseEvents(true)` keep the window out
   * of the focus chain and out of the event stream; the source window holds the
   * pointer capture that IS the drag, so either one missing hands the gesture to
   * a window that has nothing to do with it.
   */
  it('builds a window that cannot take the focus the drag depends on', () => {
    const r = recipe();
    expect(r).toMatch(/focusable:\s*false/);
    expect(r).toMatch(/ghost\.setIgnoreMouseEvents\(true\)/);
    // `show: false` at construction, then `showInactive()`. A bare `show()`
    // ACTIVATES the window, which is the same focus theft by another route —
    // so the string must not appear as a call on the ghost at all.
    expect(r).toMatch(/show:\s*false/);
    expect(r).toMatch(/ghost\.showInactive\(\)/);
    expect(r).not.toMatch(/ghost\.show\(\)/);
    expect(r).toMatch(/acceptFirstMouse:\s*false/);
  });

  /**
   * ⚠ The recipe is the launcher's, and the launcher HAS `vibrancy`. Copying
   * that line across is the plausible mistake, which is why this asserts its
   * absence here while the file keeps it elsewhere.
   */
  it('omits the launcher options that the design flags as hazards', () => {
    expect(recipe()).not.toContain('vibrancy');
    // The larger window carries transparent slack for the outline and the CSS
    // shadow, so a NATIVE shadow would trace that rectangle instead of the chip.
    expect(recipe()).toMatch(/hasShadow:\s*false/);
    // `vibrancy` still exists in the file — otherwise the assertion above would
    // pass against a main.ts that had lost the launcher entirely.
    expect(source).toContain('vibrancy');
  });

  /** Transparency is not portable; the design gates it and so must the code. */
  it('gates transparency on darwin and gives the other platforms a ground', () => {
    const r = recipe();
    expect(r).toMatch(/const transparent = process\.platform === 'darwin'/);
    expect(r).toMatch(/backgroundColor: transparent \? '#00000000' : spec\.style\.background/);
  });

  /** A chip with nothing to say gets none of the app's IPC doors. */
  it('gives the ghost page no preload and no node', () => {
    expect(code(recipe())).not.toContain('preload');
    expect(recipe()).toMatch(/nodeIntegration:\s*false/);
    expect(recipe()).toMatch(/sandbox:\s*true/);
    // `preload` still reaches other windows in this file, so the assertion
    // above is measuring the recipe and not a spelling that left main.ts.
    expect(code(source)).toContain('preload');
  });

  /**
   * Issue #75's "no new IPC": the screen point already reaches main on every
   * move. It must arrive through `screenPointFromWire` (the `{screenX,screenY}`
   * vs `{x,y}` mismatch that silently killed the feature once) and be converted
   * by `normalizeToDip` before it becomes a window position.
   */
  it('positions the ghost from the existing move handler, after normalizeToDip', () => {
    const move = block("ipcMain.on('tab-drag:move'", '\n  });');
    expect(move).toContain('screenPointFromWire');
    expect(move).toMatch(/dragGhostWindows\.follow\(source\.id, normalizeToDip\(/);
    // `detach` only: a `merge` already paints a caret saying where the tab lands.
    expect(move).toMatch(/dragGhostWindows\.release\(`phase \$\{phase\.kind\}`\)/);
  });

  /**
   * ⚠ Every exit, not just the tidy one. A ghost is click-through and
   * always-on-top, so one left behind sits over every other app with no gesture
   * behind it and no way for the user to dismiss it.
   */
  it('releases the ghost on each of the three ways a drag can end', () => {
    expect(block("ipcMain.on('tab-drag:end'", '\n  });')).toContain('dragGhostWindows.release(');
    // Including the source dying: crash, reload and close all route here.
    expect(block('function forgetWindowFromTabDrag(', '\n}')).toContain(
      'dragGhostWindows.releaseIfSource(windowId, reason)'
    );
  });

  /**
   * ⚠ **First statement, ahead of every early return.** The button is up, so the
   * ghost represents nothing whatever the handler goes on to decide. Below the
   * first `return` it would hang over the desktop for the whole 2 s merge-ack
   * window, and for good on a `noop`.
   */
  it('releases on commit before the handler can return', () => {
    const commit = code(block("ipcMain.handle('tab-drag:commit'", '\n  });'));
    const released = commit.indexOf('dragGhostWindows.release(');
    const firstReturn = commit.search(/\breturn\b/);
    expect(released, 'commit never releases the ghost').toBeGreaterThan(-1);
    expect(firstReturn, 'commit has no early return left to guard').toBeGreaterThan(-1);
    expect(released).toBeLessThan(firstReturn);
  });
});

/**
 * Spec 2.1 and 3.16: the ghost's fallback look speaks the app's one UI stack
 * (`--font-body`, the website's Arial-first stack) and draws the app's chat
 * glyph, so a torn-off tab does not change face or icon the moment the probe
 * comes back empty.
 */
describe('the ghost fallback matches the app', () => {
  it('uses the --font-body stack from main.css, byte for byte (whitespace aside)', () => {
    const css = readFileSync(join(__dirname, 'styles/main.css'), 'utf8');
    const match = css.match(/--font-body:\s*([^;]+);/);
    expect(match, 'no --font-body in main.css').not.toBeNull();
    const normalise = (value: string) => value.replace(/\s+/g, ' ').trim();
    expect(normalise(DEFAULT_GHOST_STYLE.fontFamily)).toBe(normalise(match![1]));
    // And it survives the sanitiser it is the fallback for.
    expect(sanitizeCssValue(DEFAULT_GHOST_STYLE.fontFamily, 'x')).toBe(
      normalise(DEFAULT_GHOST_STYLE.fontFamily)
    );
  });

  it('draws the chat bubble the strip draws, at the app stroke', () => {
    const html = ghostWindowHtml(
      { ...DEFAULT_GHOST_SPEC, title: 'Cohort query' },
      { transparent: true }
    );
    expect(html).toContain('stroke-width="1.5"');
    expect(html).toContain('M3 8a3.5 3.5 0 0 1 3.5-3.5h11A3.5 3.5 0 0 1 21 8v5.5');
  });
});
