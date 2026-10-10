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
  // The first placement lands at rest; only later moves slide.
  const [still, setStill] = React.useState(true);
  const selectedIndex = options.findIndex((option) => option.value === value);

  const setTrackRef = React.useCallback(
    (node: HTMLDivElement | null) => {
      trackRef.current = node;
      if (typeof ref === 'function') ref(node);
      else if (ref) ref.current = node;
    },
    [ref]
  );

  const measure = React.useCallback(() => {
    if (fill) return;
    const segment = segmentRefs.current.get(value);
    if (!segment) return;
    const width = segment.offsetWidth;
    // jsdom (and a display:none ancestor) measures 0: keep the segment's own ground then.
    if (width <= 0) return;
    setThumb((current) =>
      current && current.left === segment.offsetLeft && current.width === width
        ? current
        : { left: segment.offsetLeft, width }
    );
  }, [fill, value]);

  React.useLayoutEffect(() => {
    measure();
  }, [measure, options]);

  React.useEffect(() => {
    if (fill || typeof ResizeObserver === 'undefined') return;
    const track = trackRef.current;
    if (!track) return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(track);
    segmentRefs.current.forEach((segment) => observer.observe(segment));
    return () => observer.disconnect();
  }, [fill, measure, options]);

  const placed = fill ? selectedIndex >= 0 : thumb !== null;

  // Release the still flag one frame after the first placement, so the next move animates.
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
      : { width: thumb!.width, transform: `translateX(${thumb!.left}px)` };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Home' && event.key !== 'End') return;
    const enabled = options.filter((option) => !option.disabled);
    const target = event.key === 'Home' ? enabled[0] : enabled[enabled.length - 1];
    if (target && target.value !== value) onValueChange(target.value);
  };

  return (
    <RadioGroupPrimitive.Root
      ref={setTrackRef}
      id={id}
      name={name}
      value={value}
      onValueChange={(next) => onValueChange(next as T)}
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
      onKeyDown={onKeyDown}
    >
      <span
        aria-hidden="true"
        className="br-segmented-thumb"
        data-slot="segmented-thumb"
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
