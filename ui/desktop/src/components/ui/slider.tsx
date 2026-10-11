'use client';

import * as React from 'react';

import { cn } from '../../utils';

/**
 * Slider: one value from a short, ordered scale of named stops (owner message 13, ruling R-26):
 * the model menu's Effort (Quick, Normal, Deep). Built in house with plain React and pointer
 * events, because `@radix-ui/react-slider` is not a direct dependency and nothing may be
 * installed.
 *
 * Discrete only. Each stop is a `{ value, label }`; the value is one of the stops' values, never
 * a number between them. A choice applies at once (`onValueChange` fires on every change, during
 * a drag too), like the radio rows it replaces.
 *
 * Semantics (WAI-ARIA slider pattern): the THUMB is the control, `role="slider"`, one Tab stop,
 * `aria-valuemin` 0, `aria-valuemax` the last stop's index, `aria-valuenow` the current index and
 * `aria-valuetext` its label ("Normal"), so a screen reader speaks the word, not a number. It
 * is named by `aria-label` or `aria-labelledby`. `description` is rendered in a visually hidden
 * node the thumb points at with `aria-describedby` (merged with the caller's own, so an InfoTip's
 * id can be added), so the per-level explanations need no hover.
 *
 * Keys: Right and Up raise it, Left and Down lower it, Home and End jump to the ends, Page Up and
 * Page Down move by `pageStep` stops (a tenth of the scale, at least one). In a right-to-left
 * layout Left raises and Right lowers, because the scale runs from the right. Keys it handles
 * are stopped there, so a menu or a roving list around it does not move as well; a key with
 * Alt, Ctrl or Meta held passes through untouched.
 *
 * Pointer: a press anywhere on the slider moves the thumb to the nearest stop and keeps the
 * pointer captured, so a drag snaps from stop to stop until release. The press focuses the thumb
 * without its keyboard focus edge (principle 10 of the vision checklist: no ring after a click);
 * the next key brings the edge back.
 *
 * Geometry and colour are authored CSS (`.br-slider*` in `main.css`, `@ws WS-PRIMITIVES`), so
 * they cannot fail to generate the way a newly written utility can:
 *   - a 24px tall hit target; the rail is inset 8px each side so the 16px thumb never overhangs
 *     the box at either end;
 *   - a 4px track on `--control-track-off`, a 6px mark at every stop in `--border-control`
 *     (3:1 on the menu and card grounds);
 *   - the filled part of the track, and the marks it covers, in `--background-accent`;
 *   - a 16px thumb in `--background-accent`, cut out of the track by a 2px ring of the ground it
 *     sits on. Keyboard focus draws D-15's `--border-focus` edge around that ring: a 16px accent
 *     disc has no surface to shift, so the edge goes where it contrasts with the ground;
 *   - disabled: 50% and `not-allowed`, and the thumb leaves the Tab order;
 *   - forced colours: the track, marks, fill and thumb are redrawn in system colours.
 *
 * Motion (principle 9): only the thumb's `transform` and the fill's `clip-path` move, over
 * `--dur-fast-max` with `--ease-out`, the same curve for both so the fill's end stays under the
 * thumb's centre. Nothing geometric transitions: the position is a percentage of the rail, so a
 * menu that changes width moves the thumb with it, at rest. A mount lands at rest (CSS starts no
 * transition on insertion), and so does a value that changes WITHOUT the person (another window,
 * a store that loads late): the render that shows it carries `data-motion-still`, set during
 * that render as the Switch does (see `switch.tsx`, V4-01) and released two frames later.
 * Reduced motion removes the transitions.
 */

export interface SliderStep<T extends string = string> {
  value: T;
  /** The stop's name, spoken as `aria-valuetext` ("Normal"). */
  label: string;
}

export interface SliderProps<T extends string = string> {
  /** The stops, in order from the low end. */
  steps: ReadonlyArray<SliderStep<T>>;
  /** One of the stops' values. An unknown value shows the first stop. */
  value: T;
  onValueChange: (value: T) => void;
  'aria-label'?: string;
  'aria-labelledby'?: string;
  /** More ids to describe the thumb, after `description`'s (an InfoTip's id, for instance). */
  'aria-describedby'?: string;
  /** What the scale means, read after the value. Never shown: it is the accessible description. */
  description?: string;
  disabled?: boolean;
  /** The layout's direction. Read from the page when not given. */
  dir?: 'ltr' | 'rtl';
  /** Stops moved by Page Up and Page Down. Default: a tenth of the scale, at least one. */
  pageStep?: number;
  /** The thumb's id (the element with `role="slider"`). */
  id?: string;
  className?: string;
  'data-testid'?: string;
}

/** How long after the person moved it a change of `value` still counts as theirs (a slow save). */
export const SLIDER_TOUCH_WINDOW_MS = 4000;

function clampIndex(index: number, count: number): number {
  return Math.min(Math.max(index, 0), Math.max(count - 1, 0));
}

function SliderInner<T extends string>(
  {
    steps,
    value,
    onValueChange,
    description,
    disabled = false,
    dir,
    pageStep,
    id,
    className,
    'aria-label': ariaLabel,
    'aria-labelledby': ariaLabelledBy,
    'aria-describedby': ariaDescribedBy,
    'data-testid': testId,
  }: SliderProps<T>,
  ref: React.ForwardedRef<HTMLSpanElement>
) {
  const count = steps.length;
  const found = steps.findIndex((step) => step.value === value);
  const index = found >= 0 ? found : 0;
  const fraction = count > 1 ? index / (count - 1) : 0;

  const rootRef = React.useRef<HTMLDivElement | null>(null);
  const railRef = React.useRef<HTMLDivElement | null>(null);
  const thumbRef = React.useRef<HTMLSpanElement | null>(null);
  const setThumbRef = React.useCallback(
    (node: HTMLSpanElement | null) => {
      thumbRef.current = node;
      if (typeof ref === 'function') ref(node);
      else if (ref) ref.current = node;
    },
    [ref]
  );

  const reactId = React.useId();
  const descriptionId = description
    ? `br-slider-${reactId.replace(/[^a-zA-Z0-9_-]/g, '')}-description`
    : undefined;
  const describedBy = [descriptionId, ariaDescribedBy].filter(Boolean).join(' ') || undefined;

  // When the person last moved it, and the index they asked for and the parent has not shown yet
  // (so a drag across one stop asks once, not on every pointer move).
  const touchedAt = React.useRef<number | null>(null);
  const requested = React.useRef<number | null>(null);
  const dragging = React.useRef(false);
  const [isDragging, setIsDragging] = React.useState(false);
  // True from a press until the next key: the thumb is focused but draws no keyboard edge.
  const [pointerFocus, setPointerFocus] = React.useState(false);

  // The index the last render showed, and a token (> 0) while a change that was NOT the person's
  // is held at rest. Derived during render, so the new position and `data-motion-still` reach the
  // DOM in the same commit (the Switch's V4-01 lesson: a flag set one commit later lets any style
  // read in between start the transition).
  const [shown, setShown] = React.useState(index);
  const [still, setStill] = React.useState(0);
  if (index !== shown) {
    setShown(index);
    const touched =
      touchedAt.current !== null && Date.now() - touchedAt.current < SLIDER_TOUCH_WINDOW_MS;
    setStill(touched ? 0 : still + 1);
  }

  React.useLayoutEffect(() => {
    // The change this render showed has been judged; the next one is judged afresh.
    touchedAt.current = null;
    requested.current = null;
  }, [index]);

  React.useEffect(() => {
    if (!still) return;
    // Two frames: the first is styled with the flag (the snap), the second releases it.
    let second = 0;
    const first = window.requestAnimationFrame(() => {
      second = window.requestAnimationFrame(() => setStill(0));
    });
    return () => {
      window.cancelAnimationFrame(first);
      window.cancelAnimationFrame(second);
    };
  }, [still]);

  const isRtl = (): boolean => {
    if (dir) return dir === 'rtl';
    const root = rootRef.current;
    return root ? window.getComputedStyle(root).direction === 'rtl' : false;
  };

  const commit = (next: number) => {
    if (count === 0) return;
    const target = clampIndex(next, count);
    // During a drag, compare with what this drag last asked for; otherwise with what is shown.
    const current = dragging.current && requested.current !== null ? requested.current : index;
    if (target === current) return;
    requested.current = target;
    touchedAt.current = Date.now();
    onValueChange(steps[target].value);
  };

  const indexAt = (clientX: number): number | null => {
    const rail = railRef.current;
    if (!rail || count === 0) return null;
    const box = rail.getBoundingClientRect();
    // jsdom (and a display:none ancestor) measures 0: nothing to aim at.
    if (box.width <= 0) return null;
    const along = isRtl() ? box.right - clientX : clientX - box.left;
    return clampIndex(Math.round((along / box.width) * (count - 1)), count);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLSpanElement>) => {
    if (pointerFocus) setPointerFocus(false);
    if (disabled || count === 0) return;
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const page = Math.max(1, Math.round(pageStep ?? Math.ceil((count - 1) / 10)));
    const forward = isRtl() ? -1 : 1;
    let next: number;
    switch (event.key) {
      case 'ArrowRight':
        next = index + forward;
        break;
      case 'ArrowLeft':
        next = index - forward;
        break;
      case 'ArrowUp':
        next = index + 1;
        break;
      case 'ArrowDown':
        next = index - 1;
        break;
      case 'PageUp':
        next = index + page;
        break;
      case 'PageDown':
        next = index - page;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = count - 1;
        break;
      default:
        return;
    }
    // Handled even at an end: the key must not scroll the page or move a surrounding menu.
    event.preventDefault();
    event.stopPropagation();
    commit(next);
  };

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (disabled || count === 0) return;
    // The primary button only; a Ctrl-click is a secondary click on macOS.
    if (event.pointerType === 'mouse' && (event.button !== 0 || event.ctrlKey)) return;
    // No native focus or text selection: the thumb is focused below, without its edge.
    event.preventDefault();
    const root = event.currentTarget;
    try {
      root.setPointerCapture?.(event.pointerId);
    } catch {
      // A synthetic pointer (a test, an automation) has no capture to take; the press still sets.
    }
    dragging.current = true;
    setIsDragging(true);
    setPointerFocus(true);
    thumbRef.current?.focus({ preventScroll: true });
    const next = indexAt(event.clientX);
    if (next !== null) commit(next);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging.current) return;
    const next = indexAt(event.clientX);
    if (next !== null) commit(next);
  };

  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging.current) return;
    dragging.current = false;
    requested.current = null;
    setIsDragging(false);
    const root = event.currentTarget;
    try {
      if (root.hasPointerCapture?.(event.pointerId)) root.releasePointerCapture(event.pointerId);
    } catch {
      // Already released.
    }
  };

  const marks = (filled: boolean) =>
    steps.map((step, stepIndex) => (
      <span
        key={step.value}
        className="br-slider-mark"
        data-slot={filled ? 'slider-fill-mark' : 'slider-mark'}
        data-value={step.value}
        style={{ insetInlineStart: `${count > 1 ? (stepIndex / (count - 1)) * 100 : 0}%` }}
      />
    ));

  const stillFlag = still > 0 ? '' : undefined;

  return (
    <div
      ref={rootRef}
      dir={dir}
      className={cn('br-slider', className)}
      data-slot="slider"
      data-testid={testId}
      data-disabled={disabled ? '' : undefined}
      data-dragging={isDragging ? '' : undefined}
      style={{ '--br-slider-fraction': fraction } as React.CSSProperties}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onLostPointerCapture={endDrag}
    >
      <div ref={railRef} className="br-slider-rail" data-slot="slider-rail">
        <div aria-hidden="true" className="br-slider-track" data-slot="slider-track">
          {marks(false)}
        </div>
        <div
          aria-hidden="true"
          className="br-slider-fill"
          data-slot="slider-fill"
          data-motion-layout=""
          data-motion-still={stillFlag}
        >
          {marks(true)}
        </div>
        <div
          className="br-slider-thumb-slot"
          data-slot="slider-thumb-slot"
          data-motion-layout=""
          data-motion-still={stillFlag}
        >
          <span
            ref={setThumbRef}
            id={id}
            role="slider"
            tabIndex={disabled ? undefined : 0}
            aria-label={ariaLabel}
            aria-labelledby={ariaLabelledBy}
            aria-describedby={describedBy}
            aria-valuemin={0}
            aria-valuemax={Math.max(count - 1, 0)}
            aria-valuenow={index}
            aria-valuetext={steps[index]?.label}
            aria-orientation="horizontal"
            aria-disabled={disabled ? 'true' : undefined}
            className="br-slider-thumb"
            data-slot="slider-thumb"
            data-pointer-focus={pointerFocus ? '' : undefined}
            onKeyDown={onKeyDown}
            onBlur={() => setPointerFocus(false)}
          />
        </div>
      </div>
      {description ? (
        <span id={descriptionId} className="sr-only" data-slot="slider-description">
          {description}
        </span>
      ) : null}
    </div>
  );
}

export const Slider = React.forwardRef(SliderInner) as <T extends string>(
  props: SliderProps<T> & { ref?: React.ForwardedRef<HTMLSpanElement> }
) => ReturnType<typeof SliderInner>;
