/**
 * The motion tokens, as numbers JavaScript can use.
 *
 * `styles/main.css` is the source of truth: `--dur-*`, `--ease-out` and
 * `--ease-spring` live in its `:root` block, and every CSS rule reads them with
 * `var()`. WAAPI (`element.animate`) and FLIP code cannot read a custom property
 * in its timing options, so they used to write their own literals, and those
 * drifted (300 / 250 / 125 with a private curve in the preview, 420ms with
 * another curve in the composer). This module is the one TypeScript mirror, and
 * `motion.test.ts` pins every number here to its `main.css` declaration, so the
 * two cannot disagree.
 *
 * It also replaces the copies of `prefersReducedMotion()` that each animated
 * component carried, and adds `isWindowResizing()`: layout snaps while the
 * window, a split or the sidebar is being resized (spec principle 9), so a JS
 * animation asks this before it starts.
 *
 * The rules (docs/design/codex-simplicity-redesign/implementation-spec.md §2.4):
 * - one curve (`EASE_OUT`) for everything; `EASE_SPRING` only for transforms of
 *   physical gestures (tab slide and settle, drag lift, the segmented thumb);
 * - exits take a faster tier than entrances and travel half the distance;
 * - animate `transform`, `opacity` and `clip-path` only, outside the named
 *   exceptions;
 * - a surface that mounts because of navigation, a reload, a tab switch or a
 *   history load appears at rest.
 */

/** Durations in milliseconds. Each equals the `--dur-*` token of the same tier. */
export const DUR = {
  /** `--dur-fast-min`: row and menu-item hover, row-action reveal, tooltip exit. */
  fastMin: 95,
  /** `--dur-fast`: control state changes, and every exit of a small surface. */
  fast: 125,
  /** `--dur-fast-max`: entrances of small surfaces, tab enter, crossfade in. */
  fastMax: 175,
  /** `--dur-med-min`: dialog enter, progress fill, side and stack crossfade. */
  medMin: 250,
  /** `--dur-med`: panels (preview, summary rail, conversation glide, sidebar). */
  med: 300,
  /** `--dur-slow`: the period unit for ambient loops only (`n × slow`). */
  slow: 525,
} as const;

export type MotionDuration = keyof typeof DUR;

/** `--ease-out`: the one curve. A strong decelerate with no overshoot. */
export const EASE_OUT = 'cubic-bezier(0.24, 1, 0.4, 1)';

/** `--ease-spring`: transforms of physical gestures only, never colour or opacity. */
export const EASE_SPRING = 'cubic-bezier(0.34, 1.56, 0.64, 1)';

/**
 * Constants the motion rules are written with. They are not CSS tokens; the
 * authored CSS writes the same numbers into the rules that use them (for example
 * `.br-enter` travels `TRAVEL.small`).
 */
export const MOTION = {
  /** Small surfaces (menus, popovers, context menus) pop in from this scale. */
  popScale: 0.97,
  /** Dialogs pop in from this scale. */
  dialogScale: 0.98,
  /** Stagger between list items that enter together. */
  staggerMs: 30,
  /** At most this many items animate in one batch; the rest appear at rest. */
  staggerMax: 5,
  /** The resize settle: layout motion resumes this long after the last resize event. */
  resizeSettleMs: 180,
  /** A FLIP glide shorter than this snaps instead (a jiggle reads as a glitch). */
  minGlidePx: 8,
} as const;

/** Travel distances in px. Exits travel half of an entrance. */
export const TRAVEL = {
  /** Menus and popovers toward their trigger, a row insert (`.br-enter`). */
  small: 4,
  /** Crossfade rise, a covering pane, a toast, rail content. */
  medium: 8,
  /** The preview body entering. */
  previewIn: 32,
  /** The preview body leaving. */
  previewOut: 16,
} as const;

/** Body classes that mean "geometry is being dragged right now". */
export const RESIZING_CLASSES = [
  'biorouter-window-resizing',
  'biorouter-sidebar-resizing',
] as const;

/**
 * True when the person asked the OS to reduce motion. Safe to call where
 * `matchMedia` does not exist (jsdom without a stub, a worker): that answers
 * false, the same as a machine with no preference.
 */
export function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

/**
 * True while the window or the sidebar is being resized: `AppLayout` sets
 * `biorouter-window-resizing` on `<body>` and clears it 180ms after the last
 * resize event, and the sidebar's drag handle sets `biorouter-sidebar-resizing`.
 * A JS animation that moves layout-driven geometry must not start while this is
 * true; it snaps instead (CSS gets the same rule from `[data-motion-layout]`).
 */
export function isWindowResizing(): boolean {
  if (typeof document === 'undefined' || !document.body) return false;
  const { classList } = document.body;
  return RESIZING_CLASSES.some((name) => classList.contains(name));
}

/**
 * The duration to use for a JS animation: the tier's value, or 0 when the
 * person prefers reduced motion. A caller that would rather keep a fade under
 * reduced motion (opacity is not movement) passes `{ keepUnderReducedMotion }`.
 */
export function motionDuration(
  tier: MotionDuration,
  options: { keepUnderReducedMotion?: boolean } = {}
): number {
  if (!options.keepUnderReducedMotion && prefersReducedMotion()) return 0;
  return DUR[tier];
}
