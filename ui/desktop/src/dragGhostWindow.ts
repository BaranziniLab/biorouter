import type { Point, Rect } from './windowDrag';

/**
 * The torn-off tab's ghost, as a REAL WINDOW — Phase 4b of
 * `docs/design/astryx-adoption/tab-tear-off-and-merge.md` (issue #75).
 *
 * A renderer paints only into its own `BrowserWindow` surface, so the `<div>`
 * ghost in `ChatDropOverlay` physically cannot follow the cursor onto the
 * desktop; `clampGhostToViewport` pins it to the edge instead, and D7 made that
 * the shipped behaviour. The only way to draw outside the app is a second,
 * transparent, always-on-top, click-through window that tracks the cursor. This
 * module is that window's rules.
 *
 * **No Electron import**, for the same reason `windowDrag.ts` has none: every
 * decision here — where the window goes, how big it is, what markup it holds,
 * when it is created and when it dies — is expressible in plain values, and
 * keeping it that way is what lets the lifecycle be unit-tested without an
 * Electron runtime. `main.ts` supplies the two Electron-shaped capabilities
 * through {@link DragGhostHost}.
 *
 * ⚠ WHAT NO TEST IN THIS REPO CAN COVER. jsdom has no layout, no pointer
 * capture, no second window and no screen, and the existing drag suites say so
 * in their own headers. The three things that decide whether this feature works
 * at all are therefore verifiable only with real OS input on a real desktop:
 *
 *   1. that the ghost window does not steal focus (which would drop the source
 *      window's pointer capture and END the gesture mid-air — the risk the whole
 *      recipe below exists to avoid);
 *   2. that it actually paints outside the source window's frame;
 *   3. that it lands on the right display under per-monitor DPI.
 *
 * What IS tested here is the arithmetic and the lifecycle state machine: one
 * window per drag, positioned at `point − grabOffset`, destroyed on every exit
 * path including a create that is still in flight. A test asserting "the ghost
 * follows the cursor" would be false comfort and is deliberately absent.
 */

// ── The ghost's measurements, as read from the live DOM ghost ───────────────

/**
 * The resolved paint values of the source window's DOM ghost.
 *
 * These are READ FROM THE RUNNING RENDERER rather than reconstructed here, and
 * that is the only way to be right: three theme families times two modes is six
 * palettes, and `--accent-bar` alone differs in all three families. A hardcoded
 * colour would be correct in Parchment light and wrong everywhere else.
 */
export interface GhostStyle {
  /** Resolved `background-color` of `.br-tab-ghost` (`--background-default`). */
  background: string;
  /** Resolved `color` (`--text-default`). */
  color: string;
  /**
   * The raw `--accent-bar` token, which the dashed detach outline is mixed from.
   *
   * NOT the ghost's computed `outline-color`: `.br-tab-ghost` TRANSITIONS
   * `outline-color` over `--dur-fast`, and the probe runs on the very frame
   * `data-detach` flips, so the computed value at that instant is still mid-fade
   * — usually fully transparent. Reading the token instead reads the value the
   * transition is heading towards.
   */
  accent: string;
  /** Resolved `border-radius` (`--radius-element`, the tab's corner). */
  radius: string;
  /** Resolved `font-family` (`--font-sans`), quotes and all. */
  fontFamily: string;
  fontSize: string;
  fontWeight: string;
  /** Resolved `box-shadow` (`--shadow-popover`). Dropped on opaque platforms. */
  shadow: string;
  /**
   * Resolved horizontal inset and icon gap, so the chip's metrics match. The inset
   * is the padding PLUS the tab's 1px edge (transparent, R-27): this page draws no
   * edge, so the edge's width is folded into its padding and the label lands at
   * the same x as in the DOM ghost.
   */
  paddingX: string;
  gap: string;
}

export interface GhostSpec {
  title: string;
  /** The DOM ghost's measured size in CSS px, which are DIP (the app sets no zoom factor). */
  width: number;
  height: number;
  /**
   * Where inside the tab the pointer grabbed it, straight off the DOM ghost's
   * `data-grab-x`/`data-grab-y`.
   *
   * This is the same offset `tornOffWindowBounds` subtracts to place the torn-off
   * WINDOW, which is why the ghost and the window it becomes land at the same
   * origin by construction rather than by two rules that have to be kept in step.
   */
  grabOffset: Point;
  style: GhostStyle;
}

/** `.br-tab`'s own `min-width`/`max-width`, so a nonsense measurement cannot make a wall-sized ghost. */
export const GHOST_MIN_WIDTH = 88;
export const GHOST_MAX_WIDTH = 190;
export const GHOST_MIN_HEIGHT = 16;
export const GHOST_MAX_HEIGHT = 96;

/**
 * Transparent padding around the chip, in DIP.
 *
 * The detached ghost wears a 2px outline (drawn OUTSIDE the border box) and
 * `--shadow-popover` (24px of blur), neither of which
 * `getBoundingClientRect()` includes. Without slack the window clips both and
 * the ghost reads as a bare rectangle.
 */
export const GHOST_TRANSPARENT_INSET = 14;

/**
 * Zero slack where the window cannot be transparent.
 *
 * `transparent` is darwin-gated throughout this app (`main.ts`'s launcher does
 * the same), so on Windows and Linux the ghost window is an OPAQUE rectangle:
 * any inset would paint a visible frame of the chip's background colour around
 * the chip. There the window IS the chip, and the dashed edge moves inside the
 * box (see {@link ghostWindowHtml}).
 */
export const GHOST_OPAQUE_INSET = 0;

/**
 * The look when the probe comes back empty — a renderer that reloaded, a
 * `.br-tab-ghost` that is not on screen, a `executeJavaScript` that threw.
 *
 * Parchment light, because it is the default theme; being wrong about the
 * palette is a cosmetic miss on a path that only opens when the real values are
 * unreadable, and it beats refusing to show a ghost at all.
 */
export const DEFAULT_GHOST_STYLE: GhostStyle = {
  background: '#ffffff',
  color: '#2a2520',
  accent: '#cf6d47',
  radius: '8px',
  // The app's one UI stack, `--font-body` in main.css (spec 2.1; the website's
  // Arial-first stack). `dragGhostWindow.test.ts` pins the two together.
  fontFamily:
    "Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
  fontSize: '13px',
  fontWeight: '400',
  shadow: '0 8px 24px rgba(0, 0, 0, 0.18)',
  paddingX: '11px',
  gap: '8px',
};

export const DEFAULT_GHOST_SPEC: GhostSpec = {
  title: '',
  width: 160,
  height: 32,
  // Half the default chip: with no measured grab point the least surprising
  // place for the ghost is centred under the cursor.
  grabOffset: { x: 80, y: 16 },
  style: DEFAULT_GHOST_STYLE,
};

// ── Sanitising what the renderer said ──────────────────────────────────────

/**
 * Everything below treats the probe result as HOSTILE, and the reason is not
 * paranoia about our own renderer: these strings are spliced into a `<style>`
 * block in a *different* window. A value that closed its rule and opened
 * another would be a stylesheet the source page authored in a window it does not
 * own. One allowlist, applied to every field, is cheaper to be sure of than any
 * argument about which fields could be influenced.
 */
const CSS_VALUE_ALLOWED = /^[a-zA-Z0-9 ,.()%#/_'"+-]*$/;
const CSS_VALUE_FORBIDDEN = /var\(|url\(|expression|@import|<|\\/i;

function balanced(value: string, open: string, close: string): boolean {
  let depth = 0;
  for (const char of value) {
    if (char === open) depth++;
    else if (char === close && --depth < 0) return false;
  }
  return depth === 0;
}

/** A CSS value that is safe to inline, or `fallback` if it is anything else. */
export function sanitizeCssValue(input: unknown, fallback: string): string {
  if (typeof input !== 'string') return fallback;
  const value = input.replace(/\s+/g, ' ').trim();
  if (!value || value.length > 200) return fallback;
  if (!CSS_VALUE_ALLOWED.test(value)) return fallback;
  if (CSS_VALUE_FORBIDDEN.test(value)) return fallback;
  // An unbalanced quote or paren swallows the rest of the stylesheet even though
  // every character in it passed the allowlist.
  if (!balanced(value, '(', ')')) return fallback;
  if ((value.match(/'/g)?.length ?? 0) % 2 !== 0) return fallback;
  if ((value.match(/"/g)?.length ?? 0) % 2 !== 0) return fallback;
  return value;
}

/**
 * A colour the ghost can actually paint.
 *
 * `getComputedStyle().getPropertyValue('--accent-bar')` is supposed to hand back
 * the substituted value, but the token is authored as `var(--color-coral-500)`
 * and a build that returns the *specified* text instead would inline a `var()`
 * naming a custom property the ghost window has never heard of — a declaration
 * that silently drops, taking the outline with it. Requiring a literal colour
 * shape closes that off wherever it lands.
 */
export function sanitizeColor(input: unknown, fallback: string): string {
  const value = sanitizeCssValue(input, '');
  if (!value) return fallback;
  if (
    !/^(#[0-9a-f]{3,8}|(rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch|color)\(|[a-z]+$)/i.test(value)
  ) {
    return fallback;
  }
  return value;
}

function clampNumber(input: unknown, min: number, max: number, fallback: number): number {
  if (typeof input !== 'number' || !Number.isFinite(input)) return fallback;
  return Math.min(Math.max(input, min), max);
}

/** Read the probe's answer into a {@link GhostSpec}, field by field, or defaults. */
export function sanitizeGhostSpec(input: unknown): GhostSpec {
  const probe = (input ?? {}) as Record<string, unknown>;
  const style = (probe.style ?? {}) as Record<string, unknown>;
  const rawTitle = typeof probe.title === 'string' ? probe.title : '';
  const width = clampNumber(
    probe.width,
    GHOST_MIN_WIDTH,
    GHOST_MAX_WIDTH,
    DEFAULT_GHOST_SPEC.width
  );
  const height = clampNumber(
    probe.height,
    GHOST_MIN_HEIGHT,
    GHOST_MAX_HEIGHT,
    DEFAULT_GHOST_SPEC.height
  );
  return {
    // Control characters out (they would corrupt the data URL), then a hard cap:
    // a tab title is at most 190px of text and the chip ellipsises anyway.
    // eslint-disable-next-line no-control-regex
    title: rawTitle.replace(/[\u0000-\u001f\u007f]/g, ' ').slice(0, 160),
    width,
    height,
    grabOffset: {
      // Clamped INTO the chip: an offset outside it would hang the ghost off the
      // cursor, and a negative one would put the cursor outside the thing it is
      // supposed to be carrying.
      x: clampNumber(probe.grabOffsetX, 0, width, width / 2),
      y: clampNumber(probe.grabOffsetY, 0, height, height / 2),
    },
    style: {
      background: sanitizeColor(style.background, DEFAULT_GHOST_STYLE.background),
      color: sanitizeColor(style.color, DEFAULT_GHOST_STYLE.color),
      accent: sanitizeColor(style.accent, DEFAULT_GHOST_STYLE.accent),
      radius: sanitizeCssValue(style.radius, DEFAULT_GHOST_STYLE.radius),
      fontFamily: sanitizeCssValue(style.fontFamily, DEFAULT_GHOST_STYLE.fontFamily),
      fontSize: sanitizeCssValue(style.fontSize, DEFAULT_GHOST_STYLE.fontSize),
      fontWeight: sanitizeCssValue(style.fontWeight, DEFAULT_GHOST_STYLE.fontWeight),
      shadow: sanitizeCssValue(style.shadow, DEFAULT_GHOST_STYLE.shadow),
      paddingX: sanitizeCssValue(style.paddingX, DEFAULT_GHOST_STYLE.paddingX),
      gap: sanitizeCssValue(style.gap, DEFAULT_GHOST_STYLE.gap),
    },
  };
}

// ── Geometry ───────────────────────────────────────────────────────────────

/**
 * Where the ghost window goes for a cursor at `point`.
 *
 * @param point the cursor in **DIP** — normalised with `normalizeToDip` before
 *   it gets here. Raw `screenX`/`screenY` are not DIP under Windows per-monitor
 *   DPI, and `BrowserWindow` bounds are; that mismatch has bitten this feature
 *   before (windowDrag.ts D4).
 * @param inset transparent slack around the chip; see {@link GHOST_TRANSPARENT_INSET}.
 *
 * `point − grabOffset` is deliberately the same expression as
 * `tornOffWindowBounds`, so the ghost's top-left IS the torn-off window's
 * top-left: the window appears exactly where the ghost was, with no second rule
 * to keep in step. NOT clamped to the work area, and that is the whole feature —
 * the ghost may hang off the edge of a display because the cursor can.
 */
export function ghostWindowBounds(point: Point, spec: GhostSpec, inset: number): Rect {
  return {
    x: Math.round(point.x - spec.grabOffset.x - inset),
    y: Math.round(point.y - spec.grabOffset.y - inset),
    width: Math.round(spec.width + inset * 2),
    height: Math.round(spec.height + inset * 2),
  };
}

// ── The page ───────────────────────────────────────────────────────────────

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * `ChatBubble` (`ENTITY_ICONS.chat`) from `components/icons/app-icons`, inlined.
 *
 * The design doc's Phase 4b sketch reuses the real component at a `#/drag-ghost`
 * route; that needs a route in `App.tsx`, and a second full renderer boot per
 * drag to go with it. A 6-line static path is the same 16px glyph without either
 * cost — and this window has no React, no preload and no app bundle, which is
 * also what keeps its creation cheap enough to sit inside a pointer gesture.
 */
const CHAT_BUBBLE_SVG =
  '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"' +
  ' stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M3 8a3.5 3.5 0 0 1 3.5-3.5h11A3.5 3.5 0 0 1 21 8v5.5a3.5 3.5 0 0 1-3.5 3.5H8.4a1.5 1.5 0 0 0-.98.37L4.25 20.1A.75.75 0 0 1 3 19.53Z"/></svg>';

/**
 * The ghost window's whole document.
 *
 * Mirrors `.br-tab` + `.br-tab-ghost[data-detach='true']` in `main.css`: flat
 * (no 2deg tilt — detached reads as a window, not a tab), the dashed accent
 * outline at 55%, `opacity: .95`, the popover shadow.
 *
 * `transparent: false` is not a degraded copy of the same page, it is a
 * different one: with an opaque window the dashed edge has to move INSIDE the
 * box (an outline would be clipped by the window frame it is drawn on), and the
 * shadow goes entirely, because a shadow with nothing to fall on is just a dark
 * smear at the chip's own edge.
 */
export function ghostWindowHtml(spec: GhostSpec, options: { transparent: boolean }): string {
  const s = spec.style;
  const edge = options.transparent
    ? `outline: 2px dashed color-mix(in srgb, ${s.accent} 55%, transparent);` +
      `box-shadow: ${s.shadow};`
    : `border: 2px dashed color-mix(in srgb, ${s.accent} 55%, transparent);`;
  const pageBackground = options.transparent ? 'transparent' : s.background;
  return `<!doctype html>
<html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
<title>drag ghost</title>
<style>
  html, body {
    margin: 0; padding: 0; height: 100%; overflow: hidden;
    background: ${pageBackground};
    cursor: default; -webkit-user-select: none; user-select: none;
  }
  body { display: flex; align-items: center; justify-content: center; }
  .ghost {
    box-sizing: border-box;
    display: flex; align-items: center; gap: ${s.gap};
    width: ${spec.width}px; height: ${spec.height}px;
    padding: 0 ${s.paddingX};
    border-radius: ${s.radius};
    background: ${s.background};
    color: ${s.color};
    font-family: ${s.fontFamily};
    font-size: ${s.fontSize};
    font-weight: ${s.fontWeight};
    opacity: 0.95;
    ${edge}
  }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  svg { flex: none; }
</style></head>
<body><div class="ghost">${CHAT_BUBBLE_SVG}<span class="label">${escapeHtml(spec.title)}</span></div></body></html>`;
}

/**
 * The page as something `loadURL` will take.
 *
 * A `data:` URL rather than a file: nothing to write, nothing to clean up, and
 * no path for a stale ghost document to be left on disk. `data:` is not a
 * network request, so the app's header rules never see it; the page carries its
 * own CSP above instead, denying everything but the inline style it needs.
 */
export function ghostWindowDataUrl(html: string): string {
  return `data:text/html;charset=utf-8,${encodeURIComponent(html)}`;
}

// ── The probe ──────────────────────────────────────────────────────────────

/**
 * Read the live DOM ghost out of the source renderer.
 *
 * WHY MAIN READS THE DOM INSTEAD OF BEING TOLD. Everything this window needs —
 * the tab's title, its measured size, the grab offset, the six theme values —
 * exists in the source renderer and nowhere else, and `tab-drag:move` carries
 * only `{screenX, screenY}`: `ChatGroupsShell` builds that object literally, so
 * extra fields cannot ride along, and adding a channel means editing `preload.ts`
 * and the shell. One `executeJavaScript`, once per drag, keeps the change inside
 * the ghost feature. If a future pass is already touching preload, moving this
 * to a proper channel is strictly better — nothing else depends on the read
 * being a DOM read.
 *
 * ⚠ IT SELECTS `.br-tab-ghost`. That class is load-bearing in `main.css` too, so
 * it is not a selector that gets renamed casually — but this file is a second
 * consumer of it, and `ChatDropOverlay.tsx` says so beside the element.
 *
 * Returns `null` (not a throw) when there is no ghost, so a drag that got here
 * some other way degrades to {@link DEFAULT_GHOST_SPEC} instead of failing.
 */
export const GHOST_PROBE_SCRIPT = `(() => {
  const el = document.querySelector('.br-tab-ghost') ||
             document.querySelector('[data-testid="chat-tab-ghost"]');
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  const root = getComputedStyle(document.documentElement);
  return {
    title: (el.textContent || '').trim(),
    width: rect.width,
    height: rect.height,
    grabOffsetX: Number(el.getAttribute('data-grab-x')),
    grabOffsetY: Number(el.getAttribute('data-grab-y')),
    style: {
      background: style.backgroundColor,
      color: style.color,
      accent: root.getPropertyValue('--accent-bar').trim(),
      radius: style.borderTopLeftRadius,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      fontWeight: style.fontWeight,
      shadow: style.boxShadow,
      paddingX: (parseFloat(style.paddingLeft) + parseFloat(style.borderLeftWidth || '0')) + 'px',
      gap: style.columnGap,
    },
  };
})()`;

// ── The lifecycle ──────────────────────────────────────────────────────────

/** One live ghost window, as much of it as the controller touches. */
export interface GhostWindowHandle {
  setPosition(x: number, y: number): void;
  /** Must be a SHOW WITHOUT ACTIVATION (`showInactive`) — see the class note. */
  show(): void;
  destroy(): void;
}

/** The Electron this module refuses to import. Satisfied by `main.ts`. */
export interface DragGhostHost {
  /** Run {@link GHOST_PROBE_SCRIPT} in the source window. `null` if unavailable. */
  probeSource(sourceWindowId: number): Promise<unknown>;
  /** Build the window, already at `bounds`, still HIDDEN. `null` if it could not be made. */
  createWindow(
    sourceWindowId: number,
    bounds: Rect,
    spec: GhostSpec
  ): Promise<GhostWindowHandle | null>;
  /**
   * Tell the source renderer whether the OS ghost is up, so it can hide its own
   * `<div>` ghost. Two ghosts at once is the visible failure this prevents.
   */
  notifySource(sourceWindowId: number, active: boolean): void;
  onError?(message: string, error: unknown): void;
}

export interface DragGhostControllerOptions {
  /** {@link GHOST_TRANSPARENT_INSET} on darwin, {@link GHOST_OPAQUE_INSET} elsewhere. */
  inset: number;
}

/**
 * Create at most one ghost window per drag, keep it under the cursor, and be
 * certain it dies.
 *
 * THE FOCUS RULE IS THE FEATURE. The source window holds the pointer capture
 * that makes the whole gesture work; raising or focusing ANY window drops it and
 * the drag ends in mid-air. That is why {@link GhostWindowHandle.show} is
 * documented as a show-WITHOUT-activation, and why `main.ts` builds the window
 * `focusable: false` with `setIgnoreMouseEvents(true)`. The same lesson is
 * already recorded on the merge preview, which deliberately does not raise its
 * target (`windowDrag.ts`, `showPreview`).
 *
 * EVERY EXIT PATH DESTROYS, including one that has not finished being born. The
 * window is created asynchronously (a probe, then a page load), and the drag can
 * end during either await — so a generation counter marks any in-flight create
 * as stale and the window it produces is destroyed on arrival rather than left
 * floating over the desktop with no gesture behind it. `forgetWindow` covers the
 * case the renderer cannot report at all: a source that crashed or reloaded
 * sends no `tab-drag:end`, exactly as D4 found for the merge caret.
 */
export class DragGhostWindowController {
  private readonly host: DragGhostHost;
  private readonly inset: number;

  private sourceWindowId: number | null = null;
  private handle: GhostWindowHandle | null = null;
  private creating = false;
  /** Bumped by every release; an in-flight create whose generation is stale self-destructs. */
  private generation = 0;
  private lastPoint: Point = { x: 0, y: 0 };
  /** The spec the live window was built from — its size is what `position` offsets against. */
  private currentSpec: GhostSpec | null = null;

  constructor(host: DragGhostHost, options: DragGhostControllerOptions) {
    this.host = host;
    this.inset = options.inset;
  }

  /** The window currently being dragged FROM, or `null` when no ghost is in play. */
  get sourceWindow(): number | null {
    return this.sourceWindowId;
  }
  /** A ghost window exists and has been shown. */
  get isShowing(): boolean {
    return this.handle !== null;
  }
  /** A create is in flight. */
  get isPending(): boolean {
    return this.creating;
  }

  /**
   * The cursor is outside every chat window, at `point` (DIP). Put the ghost
   * there — creating it on the first such move and only repositioning after.
   *
   * Most drags never leave the window, so nothing here runs for them at all.
   */
  follow(sourceWindowId: number, point: Point): void {
    if (this.sourceWindowId !== null && this.sourceWindowId !== sourceWindowId) {
      // A second window started a drag while ours was up (the first window's
      // `tab-drag:end` was lost). The old ghost belongs to nobody now.
      this.release('another window took over the drag');
    }
    this.sourceWindowId = sourceWindowId;
    this.lastPoint = { x: point.x, y: point.y };
    if (this.handle) {
      this.position(this.handle);
      return;
    }
    if (this.creating) return;
    this.creating = true;
    void this.create(sourceWindowId, this.generation);
  }

  /** The gesture is over, however it ended: no ghost, and the DOM ghost comes back. */
  release(reason: string): void {
    const handle = this.handle;
    const source = this.sourceWindowId;
    const wasInPlay = handle !== null || this.creating;
    this.generation++;
    this.creating = false;
    this.handle = null;
    this.sourceWindowId = null;
    // The next drag is a different tab with a different title, so a different
    // width and grab offset: carrying this one's spec over would place that
    // ghost against the wrong measurements for its first frame.
    this.currentSpec = null;
    if (handle) {
      try {
        handle.destroy();
      } catch (error) {
        this.host.onError?.(`ghost window destroy failed (${reason})`, error);
      }
    }
    // Unconditionally when anything was in play, including a create that never
    // finished: the renderer must never be left with its own ghost hidden and no
    // OS ghost to replace it — that is an invisible tab in mid-drag.
    if (wasInPlay && source !== null) {
      this.host.notifySource(source, false);
    }
  }

  /**
   * A window is gone — closed, crashed, or its document replaced. Drop the ghost
   * only if that window is the one holding it.
   */
  releaseIfSource(windowId: number, reason: string): void {
    if (this.sourceWindowId !== windowId) return;
    this.release(reason);
  }

  private position(handle: GhostWindowHandle): void {
    const bounds = ghostWindowBounds(this.lastPoint, this.specOrDefault(), this.inset);
    try {
      handle.setPosition(bounds.x, bounds.y);
    } catch (error) {
      this.host.onError?.('ghost window move failed', error);
    }
  }

  private specOrDefault(): GhostSpec {
    return this.currentSpec ?? DEFAULT_GHOST_SPEC;
  }

  private async create(sourceWindowId: number, generation: number): Promise<void> {
    try {
      const probe = await this.host.probeSource(sourceWindowId);
      if (generation !== this.generation) return;
      const spec = sanitizeGhostSpec(probe);
      const handle = await this.host.createWindow(
        sourceWindowId,
        ghostWindowBounds(this.lastPoint, spec, this.inset),
        spec
      );
      if (!handle) return;
      if (generation !== this.generation) {
        // The drag ended while the window was loading. It has never been shown;
        // destroying it here is the only thing standing between a cancelled
        // gesture and a ghost stranded on the desktop.
        handle.destroy();
        return;
      }
      this.currentSpec = spec;
      this.handle = handle;
      // Re-position before showing: the cursor has moved since `bounds` was
      // computed, and showing at a stale point is a visible jump.
      this.position(handle);
      handle.show();
      this.host.notifySource(sourceWindowId, true);
    } catch (error) {
      this.host.onError?.('ghost window creation failed', error);
    } finally {
      if (generation === this.generation) this.creating = false;
    }
  }
}
