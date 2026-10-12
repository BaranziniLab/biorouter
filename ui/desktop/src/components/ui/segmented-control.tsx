'use client';

import * as React from 'react';
import * as RadioGroupPrimitive from '@radix-ui/react-radio-group';

import { cn } from '../../utils';

/**
 * SegmentedControl: one of 2 to 4 short options that need no explanation (spec 2.6, principle
 * 5): Light / Dark / System, a theme family, a text size, Preview / Raw, a usage range.
 *
 * A value, not a set of panels, so it is a `radiogroup` (Radix RadioGroup): one Tab stop, arrow
 * keys move and select, Home and End jump to the ends. Panel switching stays underline `Tabs`.
 *
 * Geometry (authored in `main.css`, `.br-segmented*`): a 28px track with 2px padding on
 * `--background-muted`, 24px segments at radius 6 in `text-secondary`, muted ink at rest and
 * default ink when selected. ONE absolutely positioned thumb (`--background-default`, raised
 * shadow, inset hairline) sits under the selected segment and slides with `translate` (175ms
 * `--ease-spring`) and `width` (175ms `--ease-out`). It never uses the accent.
 *
 * Placement: `fill` mode computes it (`index × 100% / n`); `fit` mode measures the selected
 * segment with a `ResizeObserver`. Until the first measurement the selected segment paints its
 * own ground and the thumb is hidden, and the first placement never animates
 * (`data-motion-still`), so a mount does not slide in from x=0.
 *
 * The value follows the keyboard (vision round 1, V4-07). Radix checks a radio that an arrow key
 * focused only if its document listener still believes an arrow is held when the focus lands,
 * and the focus lands in a `setTimeout`: a keyup that arrives first (automation, assistive
 * technology, a quick tap under load) left focus on the next segment with the old one checked.
 * So the group remembers an arrow key itself, and the segment that focus then lands on is chosen;
 * a change Radix also asks for is asked for once.
 *
 * It glides only when the PERSON picks another segment (principle 9). A box that moved because
 * the layout did (a resize, a font arriving, a label changing) is measured by the observer and
 * lands at rest, and `data-motion-layout` stops the glide while the window or the sidebar is
 * being dragged.
 */

export interface SegmentedOption<T extends string = string> {
  value: T;
  label: React.ReactNode;
  /** An optional 14px glyph before the label. */
  icon?: React.ComponentType<{ className?: string; size?: number; 'aria-hidden'?: boolean }>;
  /** An optional 10px colour swatch before the label (a theme family). */
  swatch?: string;
  testId?: string;
  /** The segment's accessible name, when `label` is an icon or abbreviated. */
  ariaLabel?: string;
  disabled?: boolean;
}

export interface SegmentedControlProps<T extends string = string> {
  options: ReadonlyArray<SegmentedOption<T>>;
  value: T;
  onValueChange: (value: T) => void;
  /** Equal columns that fill the track, instead of content-width segments. */
  fill?: boolean;
  disabled?: boolean;
  'aria-label'?: string;
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  id?: string;
  name?: string;
  className?: string;
  'data-testid'?: string;
}

type ThumbBox = { left: number; width: number };

const ARROW_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight']);

function SegmentedControlInner<T extends string>(
  {
    options,
    value,
    onValueChange,
    fill = false,
    disabled,
    className,
    id,
    name,
    'aria-label': ariaLabel,
    'aria-labelledby': ariaLabelledBy,
    'aria-describedby': ariaDescribedBy,
    'data-testid': testId,
  }: SegmentedControlProps<T>,
  ref: React.ForwardedRef<HTMLDivElement>
) {
  const trackRef = React.useRef<HTMLDivElement | null>(null);
  const segmentRefs = React.useRef(new Map<string, HTMLButtonElement>());
  const [thumb, setThumb] = React.useState<ThumbBox | null>(null);
  const thumbRef = React.useRef<ThumbBox | null>(null);
  // The first placement lands at rest; only later moves slide, and only the person's.
  const [still, setStill] = React.useState(true);
  const selectedIndex = options.findIndex((option) => option.value === value);

  // The value last asked for (or shown), so a change both Radix and the arrow handling ask for is
  // asked for once; and whether an arrow key moved focus that has not landed yet.
  const requested = React.useRef(value);
  const arrowPending = React.useRef(false);
  React.useLayoutEffect(() => {
    requested.current = value;
  }, [value]);
  const change = (next: T) => {
    if (next === requested.current) return;
    requested.current = next;
    onValueChange(next);
  };

  const setTrackRef = React.useCallback(
    (node: HTMLDivElement | null) => {
      trackRef.current = node;
      if (typeof ref === 'function') ref(node);
      else if (ref) ref.current = node;
    },
    [ref]
  );

  /**
   * `fromLayout` marks a measurement the observer took: the box moved because the layout did,
   * not because the person chose another segment, so the thumb snaps there instead of gliding.
   */
  const measure = React.useCallback(
    (fromLayout: boolean) => {
      if (fill) return;
      const segment = segmentRefs.current.get(value);
      if (!segment) return;
      const width = segment.offsetWidth;
      // jsdom (and a display:none ancestor) measures 0: keep the segment's own ground then.
      if (width <= 0) return;
      const next = { left: segment.offsetLeft, width };
      const current = thumbRef.current;
      if (current && current.left === next.left && current.width === next.width) return;
      thumbRef.current = next;
      if (fromLayout && current) setStill(true);
      setThumb(next);
    },
    [fill, value]
  );

  React.useLayoutEffect(() => {
    measure(false);
  }, [measure, options]);

  React.useEffect(() => {
    if (fill || typeof ResizeObserver === 'undefined') return;
    const track = trackRef.current;
    if (!track) return;
    const observer = new ResizeObserver(() => measure(true));
    observer.observe(track);
    segmentRefs.current.forEach((segment) => observer.observe(segment));
    return () => observer.disconnect();
  }, [fill, measure, options]);

  const placed = fill ? selectedIndex >= 0 : thumb !== null;

  // Release the still flag one frame after a placement that was not the person's, so the next
  // pick animates.
  React.useEffect(() => {
    if (!placed || !still) return;
    const frame = window.requestAnimationFrame(() => setStill(false));
    return () => window.cancelAnimationFrame(frame);
  }, [placed, still]);

  const thumbStyle: React.CSSProperties | undefined = !placed
    ? undefined
    : fill
      ? ({
          '--br-segmented-count': options.length,
          '--br-segmented-index': selectedIndex,
        } as React.CSSProperties)
      : // `translate`, not `transform`: the thumb's transition is written on `translate` and
        // `width` (main.css), so a `transform` here snapped while only the width slid.
        ({ width: thumb!.width, translate: `${thumb!.left}px 0` } as React.CSSProperties);

  // Capture phase, so the flag is up before Radix's roving focus sees the key, whether it then
  // moves focus at once or on a timer. The segment focus lands on is chosen in its `onFocus`
  // below, whatever the keyup did in between.
  const onKeyDownCapture = (event: React.KeyboardEvent<HTMLDivElement>) => {
    arrowPending.current = ARROW_KEYS.has(event.key);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Home' && event.key !== 'End') return;
    const enabled = options.filter((option) => !option.disabled);
    const target = event.key === 'Home' ? enabled[0] : enabled[enabled.length - 1];
    if (target && target.value !== value) change(target.value);
  };

  return (
    <RadioGroupPrimitive.Root
      ref={setTrackRef}
      id={id}
      name={name}
      value={value}
      onValueChange={(next) => change(next as T)}
      disabled={disabled}
      orientation="horizontal"
      loop
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      aria-describedby={ariaDescribedBy}
      data-slot="segmented-control"
      data-testid={testId}
      data-fill={fill ? 'true' : undefined}
      data-placed={placed ? 'true' : 'false'}
      className={cn('br-segmented', className)}
      onKeyDownCapture={onKeyDownCapture}
      onKeyDown={onKeyDown}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          arrowPending.current = false;
        }
      }}
    >
      <span
        aria-hidden="true"
        className="br-segmented-thumb"
        data-slot="segmented-thumb"
        data-motion-layout=""
        data-motion-still={still ? '' : undefined}
        data-placed={placed ? 'true' : 'false'}
        style={thumbStyle}
      />
      {options.map((option) => {
        const Icon = option.icon;
        return (
          <RadioGroupPrimitive.Item
            key={option.value}
            ref={(node) => {
              if (node) segmentRefs.current.set(option.value, node);
              else segmentRefs.current.delete(option.value);
            }}
            value={option.value}
            disabled={option.disabled}
            aria-label={option.ariaLabel}
            data-testid={option.testId}
            data-slot="segmented-item"
            className="br-segmented-item"
            onFocus={() => {
              if (!arrowPending.current) return;
              arrowPending.current = false;
              if (!option.disabled && !disabled) change(option.value);
            }}
          >
            {option.swatch ? (
              <span
                aria-hidden="true"
                className="br-segmented-swatch br-swatch-ring"
                style={{ background: option.swatch }}
              />
            ) : null}
            {Icon ? <Icon aria-hidden size={14} className="br-segmented-icon" /> : null}
            <span className="br-segmented-label">{option.label}</span>
          </RadioGroupPrimitive.Item>
        );
      })}
    </RadioGroupPrimitive.Root>
  );
}

export const SegmentedControl = React.forwardRef(SegmentedControlInner) as <T extends string>(
  props: SegmentedControlProps<T> & { ref?: React.ForwardedRef<HTMLDivElement> }
) => ReturnType<typeof SegmentedControlInner>;
