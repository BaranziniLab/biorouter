/**
 * "The app doesn't scale with the window" — the impostor, caught by the app itself.
 *
 * A driver that calls `agent-browser set_viewport`, Playwright's
 * `setViewportSize`, or DevTools device mode applies
 * `Emulation.setDeviceMetricsOverride`, which pins the renderer's viewport to a
 * fixed size regardless of the real window. The window then grows and the
 * layout does not follow it, leaving a blank band below and to the right of the
 * page. Every measurement taken afterwards is a lie, and the report that
 * reaches a developer is "the app stopped rescaling" — which sends them into
 * the CSS, where nothing is wrong.
 *
 * This has now been diagnosed at least twice in this repo. On **2026-09-08** two
 * running instances both measured `inner 1440×900` against `outer 1638×963` and
 * `outer 1440×1000`; a clean restart of one of them, with no emulation ever
 * applied, tracked OS resizes exactly at 1200×800, 1900×1050 and 1440×1000. The
 * layout was correct throughout. The full triage — and the four other ways this
 * symptom arrives — is `docs/desktop-ui/window-scaling-regressions.md`, under
 * "Viewport emulation pins `innerWidth`".
 *
 * ⚠ **Two things this cannot see**, stated here so nobody reads its silence as an
 * all-clear. `scripts/cdp-viewport-check.mjs` is the backstop for both: it
 * measures on demand and depends on no event at all.
 *
 *   1. **A pin at exactly the window size, then orphaned.** An orphaned override
 *      (the driver that set it has detached) freezes `outerWidth` as well as
 *      `innerWidth`, so both numbers stop moving while still agreeing and there
 *      is nothing left inside the page to compare. While the setting session is
 *      still attached the case resolves itself — `outer` follows the next resize
 *      and the warning fires. Neither real recurrence took this shape: both
 *      pinned a round 1440×900 that never matched the window.
 *   2. **A `resize` that Chromium never emits.** The check runs on load and on
 *      `resize`, so it is only as reliable as that event. Measured: a fresh page
 *      pinned by a driver fires one and the warning appears; but an override
 *      *stacked on an already-emulated page* did not always emit one, and the
 *      warning stayed silent until something else caused a resize. Load plus
 *      resize is still the right trigger set — a poll would burn a timer forever
 *      to catch a case the script already covers.
 *
 * `outer` is the best reference available from inside the renderer either way;
 * the ground truth is the window manager, which only the script's caller can ask.
 *
 * So the app says so itself, in development, the moment it can see the
 * discrepancy. This module is the whole decision, and it is deliberately free of
 * React, of the DOM and of any global: a diagnosis you can only exercise by
 * launching an Electron app and pinning its viewport is a diagnosis nobody
 * re-tests. `installViewportPinWarning` below is the only part that touches a
 * window, and it decides nothing.
 *
 * Three rules govern what this may do, all for the same reason — the warning
 * fires on *tooling* state, not on user state, so it must never cost the user
 * anything:
 *
 *   * **Development only.** Never in a packaged app. A user who has never opened
 *     DevTools cannot cause this and must never be shown it.
 *   * **`console.warn`, never a toast and never a throw.** The audience is the
 *     developer or agent whose own driver left the override behind, and the
 *     console is where they are already looking.
 *   * **Once per distinct pinned size.** A `resize` handler that warns on every
 *     event buries the console under hundreds of identical lines during a single
 *     window drag, which is how a real warning stops being read.
 */

export interface ViewportMetrics {
  /** `window.innerWidth` — the renderer's viewport, which emulation overrides. */
  innerWidth: number;
  innerHeight: number;
  /** `window.outerWidth` — the real OS window, which emulation cannot touch. */
  outerWidth: number;
  outerHeight: number;
  /** `process.platform` / `window.electron.platform`: `darwin`, `win32`, `linux`. */
  platform: string;
}

/**
 * Height the window chrome may legitimately eat, on every platform.
 *
 * The real title bar is about 28px; 40 leaves room for a menu bar, a slightly
 * different OS theme, and rounding, because the cost of the two errors is not
 * symmetric. A tolerance that is too tight cries "pinned" at a window that is
 * merely framed — a false alarm in the exact place a developer is looking for a
 * layout bug, which is worse than silence. A tolerance that is too loose misses
 * a pin of under 40px, and no override anyone has ever set by hand is that
 * close to the window: the sizes that cause this are round ones like 1440×900.
 */
export const TITLE_BAR_SLACK_PX = 40;

/**
 * Extra slack for a platform whose windows have a side and bottom frame.
 *
 * macOS gets **none of it on the width axis**, and that is the sharpest signal
 * this module has: a macOS window has no side frame at all, so `outerWidth` and
 * `innerWidth` agree exactly on an unpinned window, and *any* width difference
 * is emulation. Windows and Linux draw a border of a few pixels, plus a resize
 * grip; 16 covers both without approaching the size of a real pin.
 */
export const WINDOW_FRAME_SLACK_PX = 16;

interface Slack {
  width: number;
  height: number;
}

function slackFor(platform: string): Slack {
  // The frame is on all four sides where it exists at all, so a platform that
  // pays for it on the width axis pays for it on the height axis too — on top
  // of the title bar, which every platform has.
  if (platform === 'darwin') return { width: 0, height: TITLE_BAR_SLACK_PX };
  return {
    width: WINDOW_FRAME_SLACK_PX,
    height: TITLE_BAR_SLACK_PX + WINDOW_FRAME_SLACK_PX,
  };
}

function isMeasurable(value: number): boolean {
  return Number.isFinite(value) && value > 0;
}

/**
 * The diagnosis, or `null` when the viewport plausibly follows the window.
 *
 * Pure. Returns `null` — never a false alarm — whenever it cannot tell:
 *
 *   * any dimension is zero or not finite. A minimized, hidden or not-yet-shown
 *     window reports `outer` as `0`, and treating that as "the viewport is
 *     larger than the window" would warn on a state that is not a pin at all.
 *
 * The comparison is **signed**, not absolute. Chrome only ever makes `outer`
 * larger than `inner`, so a viewport that is *bigger* than its window can only
 * have been set by emulation and is a pin on every platform, however small the
 * difference — using `Math.abs` here would have let a 10px oversize through the
 * 16px Windows frame allowance.
 */
export function describeViewportPin(metrics: ViewportMetrics): string | null {
  const { innerWidth, innerHeight, outerWidth, outerHeight, platform } = metrics;

  if (
    !isMeasurable(innerWidth) ||
    !isMeasurable(innerHeight) ||
    !isMeasurable(outerWidth) ||
    !isMeasurable(outerHeight)
  ) {
    return null;
  }

  const slack = slackFor(platform);
  const widthDelta = outerWidth - innerWidth;
  const heightDelta = outerHeight - innerHeight;
  const pinned =
    widthDelta < 0 || widthDelta > slack.width || heightDelta < 0 || heightDelta > slack.height;

  if (!pinned) return null;

  return (
    `Viewport pinned at ${innerWidth}×${innerHeight} while the window is ` +
    `${outerWidth}×${outerHeight}: a DevTools device-metrics override ` +
    `(agent-browser set_viewport, Playwright setViewportSize, or DevTools device mode) ` +
    `is holding the renderer, so the layout cannot follow the window and every ` +
    `measurement lies — this is NOT a layout bug. Diagnose with ` +
    `\`npm run cdp:viewport-check -- <cdp-port>\`; the sure cure is to restart this ` +
    `instance, because clearing the override from a different CDP session was measured ` +
    `to leave the renderer half-frozen (docs/desktop-ui/window-scaling-regressions.md).`
  );
}

/**
 * A reporter that warns at most once per distinct pinned size.
 *
 * Keyed on the *sizes*, not on "have I warned yet": a driver that pins 1440×900,
 * is cleared, and later pins 1280×800 is two separate mistakes and deserves two
 * lines, while a single drag of the window edge fires `resize` dozens of times
 * at one pinned size and deserves one. Pure apart from the `warn` sink, so the
 * dedup is testable without a DOM.
 */
export function createViewportPinReporter(
  warn: (message: string) => void
): (metrics: ViewportMetrics) => void {
  const seen = new Set<string>();
  return (metrics) => {
    const message = describeViewportPin(metrics);
    if (message === null) return;
    const key = `${metrics.innerWidth}x${metrics.innerHeight}@${metrics.outerWidth}x${metrics.outerHeight}`;
    if (seen.has(key)) return;
    seen.add(key);
    warn(message);
  };
}

/** How long the window must be still before the check runs again. */
export const VIEWPORT_PIN_DEBOUNCE_MS = 400;

interface InstallOptions {
  win?: Window;
  warn?: (message: string) => void;
  platform?: string;
  debounceMs?: number;
}

/**
 * Wire the check to a real window: once on install, and after every `resize`.
 *
 * The caller is responsible for the development guard — see `renderer.tsx`,
 * which calls this behind `import.meta.env.DEV` so the whole thing is dropped
 * from a production bundle. Returns a teardown function; the renderer never
 * calls it, but a test does.
 *
 * Debounced because `resize` fires continuously while the edge is under the
 * user's hand, and the interesting reading is the one taken after the window has
 * settled — a mid-drag sample can differ from `outer` by more than the slack for
 * a frame or two on its own.
 */
export function installViewportPinWarning(options: InstallOptions = {}): () => void {
  const win = options.win ?? (typeof window === 'undefined' ? undefined : window);
  if (!win) return () => {};

  const warn = options.warn ?? ((message: string) => console.warn(message));
  // `window.electron.platform` is the preload's `process.platform`. When it is
  // missing the fallback must be the LENIENT (framed) reading, not `darwin`:
  // guessing "no side frame" for a platform we cannot identify would turn every
  // ordinary Windows window into a false alarm, and the pins actually observed
  // here are hundreds of pixels wide — far past either tolerance.
  const platform =
    options.platform ??
    (win as Window & { electron?: { platform?: string } }).electron?.platform ??
    'unknown';
  const debounceMs = options.debounceMs ?? VIEWPORT_PIN_DEBOUNCE_MS;
  const report = createViewportPinReporter(warn);

  const check = () =>
    report({
      innerWidth: win.innerWidth,
      innerHeight: win.innerHeight,
      outerWidth: win.outerWidth,
      outerHeight: win.outerHeight,
      platform,
    });

  let timer: ReturnType<typeof setTimeout> | undefined;
  const onResize = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(check, debounceMs);
  };

  check();
  win.addEventListener('resize', onResize);

  return () => {
    if (timer !== undefined) clearTimeout(timer);
    win.removeEventListener('resize', onResize);
  };
}
