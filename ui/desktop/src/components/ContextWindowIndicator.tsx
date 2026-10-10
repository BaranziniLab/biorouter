import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronsDownUp } from './icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
import { Progress } from './ui/progress';
import { useConfig } from './ConfigContext';
import { FOOTER_COPY } from './bottom_menu/copy';
import './bottom_menu/pickers.css';

interface ContextWindowGaugeProps {
  totalTokens: number | undefined;
  tokenLimit: number;
  isTokenLimitLoaded: boolean;
  onCompact: () => void;
}

const AUTO_COMPACT_THRESHOLD_KEY = 'BIOROUTER_AUTO_COMPACT_THRESHOLD';
const AUTO_COMPACT_MIN_PCT = 20;
const AUTO_COMPACT_MAX_PCT = 90;
const AUTO_COMPACT_DEFAULT_PCT = 80;

function clampThresholdPct(v: number): number {
  if (!Number.isFinite(v)) return AUTO_COMPACT_DEFAULT_PCT;
  return Math.max(AUTO_COMPACT_MIN_PCT, Math.min(AUTO_COMPACT_MAX_PCT, Math.round(v)));
}

/** Inline bar-style context gauge. Used both as a row inside the picker
 * popover and as the popover body of the standalone
 * indicator (chat-tab mode). Icon stays neutral so it matches the rest of
 * the picker icons; the bar alone goes green → yellow → orange → red as
 * usage climbs. The bar also carries a draggable threshold marker — the
 * point at which Biorouter auto-compacts — clamped to 20–90%. */
export const ContextWindowGauge: React.FC<ContextWindowGaugeProps> = ({
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  onCompact,
}) => {
  const current = totalTokens ?? 0;
  const total = tokenLimit || 0;
  // The threshold is read and written through ConfigContext, never straight to
  // the API: the context's cached config is the copy the rest of the app reads,
  // and only a write that goes through it re-reads that cache afterwards. A
  // direct `upsertConfig` here would leave every other consumer of this
  // (non-secret, therefore cached) key on the pre-write value (#52).
  const { read: readConfigValue, upsert: upsertConfigValue } = useConfig();
  const [thresholdPct, setThresholdPct] = useState<number>(AUTO_COMPACT_DEFAULT_PCT);
  // Controlled tooltip so Radix's default focus-trigger doesn't pop the
  // hint open when the popover auto-focuses the slider on mount.
  const [tooltipOpen, setTooltipOpen] = useState(false);
  const barRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);
  const pendingPctRef = useRef<number | null>(null);

  // The unmount flush below runs after the component has stopped rendering, so
  // it cannot close over `upsertConfigValue` directly without pinning whichever
  // one it saw first. A ref keeps it on the current one.
  const upsertRef = useRef(upsertConfigValue);
  useEffect(() => {
    upsertRef.current = upsertConfigValue;
  }, [upsertConfigValue]);

  useEffect(() => {
    let cancelled = false;
    readConfigValue(AUTO_COMPACT_THRESHOLD_KEY, false)
      .then((v) => {
        if (cancelled) return;
        if (typeof v === 'number' && v > 0 && v < 1) {
          setThresholdPct(clampThresholdPct(v * 100));
        }
      })
      .catch(() => {
        /* fall back to default */
      });
    return () => {
      cancelled = true;
    };
  }, [readConfigValue]);

  // Flush any in-flight pending change if the gauge unmounts mid-drag
  // (e.g. popover closes when the user releases the mouse outside of it).
  useEffect(() => {
    return () => {
      if (pendingPctRef.current !== null) {
        const v = pendingPctRef.current;
        pendingPctRef.current = null;
        upsertRef.current(AUTO_COMPACT_THRESHOLD_KEY, v / 100, false).catch((err) => {
          console.warn('Failed to save auto-compact threshold on unmount:', err);
        });
      }
    };
  }, []);

  const persistThreshold = useCallback(
    (pctValue: number) => {
      pendingPctRef.current = null;
      upsertConfigValue(AUTO_COMPACT_THRESHOLD_KEY, pctValue / 100, false).catch((err) => {
        console.warn('Failed to save auto-compact threshold:', err);
      });
    },
    [upsertConfigValue]
  );

  // Live updates during drag don't hit the API — we only persist on release
  // so a single drag costs one POST, not dozens.
  const updateThresholdLive = (raw: number) => {
    const next = clampThresholdPct(raw);
    setThresholdPct(next);
    pendingPctRef.current = next;
  };

  const handleSliderChange = (raw: number) => {
    const next = clampThresholdPct(raw);
    setThresholdPct(next);
    persistThreshold(next);
  };

  const computePctFromClientX = useCallback((clientX: number): number => {
    const bar = barRef.current;
    if (!bar) return AUTO_COMPACT_DEFAULT_PCT;
    const rect = bar.getBoundingClientRect();
    if (rect.width <= 0) return AUTO_COMPACT_DEFAULT_PCT;
    const ratio = (clientX - rect.left) / rect.width;
    return clampThresholdPct(ratio * 100);
  }, []);

  const handleDragMove = useCallback(
    (e: MouseEvent) => {
      if (!draggingRef.current) return;
      updateThresholdLive(computePctFromClientX(e.clientX));
    },
    [computePctFromClientX]
  );

  const handleDragEnd = useCallback(() => {
    draggingRef.current = false;
    window.removeEventListener('mousemove', handleDragMove);
    window.removeEventListener('mouseup', handleDragEnd);
    if (pendingPctRef.current !== null) {
      persistThreshold(pendingPctRef.current);
    }
  }, [handleDragMove, persistThreshold]);

  const handleDragStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      draggingRef.current = true;
      // Snap to where the user clicked; persist on release.
      updateThresholdLive(computePctFromClientX(e.clientX));
      window.addEventListener('mousemove', handleDragMove);
      window.addEventListener('mouseup', handleDragEnd);
    },
    [computePctFromClientX, handleDragMove, handleDragEnd]
  );

  const handleKeyAdjust = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') {
      e.preventDefault();
      handleSliderChange(thresholdPct - (e.shiftKey ? 10 : 1));
    } else if (e.key === 'ArrowRight' || e.key === 'ArrowUp') {
      e.preventDefault();
      handleSliderChange(thresholdPct + (e.shiftKey ? 10 : 1));
    } else if (e.key === 'Home') {
      e.preventDefault();
      handleSliderChange(AUTO_COMPACT_MIN_PCT);
    } else if (e.key === 'End') {
      e.preventDefault();
      handleSliderChange(AUTO_COMPACT_MAX_PCT);
    }
  };

  useEffect(() => {
    return () => {
      window.removeEventListener('mousemove', handleDragMove);
      window.removeEventListener('mouseup', handleDragEnd);
    };
  }, [handleDragMove, handleDragEnd]);

  if (!isTokenLimitLoaded && !current) return null;
  // NO MODEL, NO GAUGE. A context window is a property of a bound model, so
  // with none there is no number to report and every number this component
  // could print would be about a model that does not exist. It read
  // "128k of 128k tokens remaining" during onboarding — beside a chip that
  // correctly said "No model yet — choose a provider" — because the composer's
  // 128k fallback was announced as a loaded limit.
  //
  // Asserted on the LIMIT rather than only on the loaded flag: the flag says
  // whether a lookup finished, and a finished lookup that found nothing is
  // exactly the case being guarded. A caller with usage but no window is a
  // state nothing can render honestly either, so it is covered by the same
  // test.
  if (total <= 0) return null;
  const ratio = Math.min(1, current / total);
  const pct = Math.round(ratio * 100);
  const overThreshold = pct >= thresholdPct;
  // Three bands, not four: the old ladder spelled `warning` twice (`ratio <= 0.75`
  // and the fallback both resolved to it), so the 0.75 rung was a no-op.
  const barTone = overThreshold ? 'danger' : ratio <= 0.5 ? 'success' : 'warning';
  return (
    <div className="flex items-center gap-3 rounded-element px-2 py-1.5">
      {/* ONE metadata size in this popover: every figure here is a reading off
          a gauge, so all of it is `supporting` in the muted ink. */}
      <span className="flex-shrink-0 text-supporting text-text-muted">
        {FOOTER_COPY.contextLabel}
      </span>
      <div className="flex-1 min-w-0 flex flex-col gap-1">
        {/* The bar holds the usage fill *and* a draggable downward triangle
            marking the auto-compact threshold. The bar is the drag target;
            mousedown anywhere on the bar (or the triangle) jumps the
            threshold to the cursor and starts a drag. */}
        {/* The threshold triangle is a SIBLING of the bar, not a child of it.
            It used to live inside the track, which forced the track to run
            `overflow-visible` so the glyph could stick out above — the one
            reason this bar could not be the shared `Progress` (an 8px pill
            that clips its fill). Both are absolutely positioned against this
            same wrapper and the wrapper is exactly as wide as the track, so
            `left: {pct}%` means the identical thing it did before; only the
            vertical origin moved, from the track's top edge to the wrapper's
            (hence -2px here rather than -12px, the wrapper's own pt-2.5). */}
        <div className="relative pt-2.5">
          <Progress
            ref={barRef}
            label={FOOTER_COPY.contextUsed}
            value={pct}
            tone={barTone}
            // A conversation with one message is still using context.
            minVisiblePercent={2}
            className="cursor-pointer"
            onMouseDown={handleDragStart}
          />
          <Tooltip open={tooltipOpen}>
            <TooltipTrigger asChild>
              {/* Hit area is larger than the 10×6 visible triangle so the
                  hover tooltip doesn't flicker when the cursor grazes the
                  triangle edges. The visible glyph is the inner element. */}
              <div
                role="slider"
                aria-label={FOOTER_COPY.thresholdLabel}
                aria-valuemin={AUTO_COMPACT_MIN_PCT}
                aria-valuemax={AUTO_COMPACT_MAX_PCT}
                aria-valuenow={thresholdPct}
                tabIndex={0}
                onKeyDown={handleKeyAdjust}
                onMouseDown={handleDragStart}
                onMouseEnter={() => setTooltipOpen(true)}
                onMouseLeave={() => {
                  if (!draggingRef.current) setTooltipOpen(false);
                }}
                className="absolute flex items-end justify-center cursor-ew-resize"
                style={{
                  left: `${thresholdPct}%`,
                  top: '-2px',
                  width: '22px',
                  height: '18px',
                  transform: 'translateX(-50%)',
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    width: 0,
                    height: 0,
                    borderLeft: '5px solid transparent',
                    borderRight: '5px solid transparent',
                    borderTop: '6px solid currentColor',
                    color: 'var(--color-text-default, currentColor)',
                    pointerEvents: 'none',
                  }}
                />
              </div>
            </TooltipTrigger>
            <TooltipContent side="top">{FOOTER_COPY.thresholdTooltip(thresholdPct)}</TooltipContent>
          </Tooltip>
        </div>
        <div className="flex items-center justify-between text-supporting text-text-muted tabular-nums">
          <span>
            {fmt(current)} / {fmt(total)}
          </span>
          <span>{pct}%</span>
        </div>
      </div>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="flex flex-shrink-0">
            <button
              type="button"
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                onCompact();
              }}
              disabled={current === 0}
              aria-label={current === 0 ? FOOTER_COPY.nothingToCompact : FOOTER_COPY.compact}
              className={`flex size-control-sm items-center justify-center rounded-element transition-colors ${current === 0 ? 'cursor-not-allowed text-text-muted opacity-50' : 'tint-interactive cursor-pointer text-text-muted hover:text-text-default'}`}
            >
              <ChevronsDownUp data-testid="compact-conversation-icon" className="size-4" />
            </button>
          </span>
        </TooltipTrigger>
        <TooltipContent side="top">
          {current === 0 ? FOOTER_COPY.nothingToCompact : FOOTER_COPY.compact}
        </TooltipContent>
      </Tooltip>
    </div>
  );
};

function fmt(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return m % 1 === 0 ? `${m.toFixed(0)}M` : `${m.toFixed(1)}M`;
  }
  if (n >= 1000) {
    const k = n / 1000;
    return k % 1 === 0 ? `${k.toFixed(0)}k` : `${k.toFixed(1)}k`;
  }
  return n.toString();
}

interface ContextWindowIndicatorProps extends ContextWindowGaugeProps {
  /**
   * Print "N% left" beside the ring once less than half the window remains
   * (spec 3.7). Above half the ring says enough on its own, and a figure that
   * reads "100%" beside a fresh chat says nothing at all.
   *
   * ⚠ The figure is the REMAINING share, because the arc the ring draws is the
   * remaining arc (`strokeDashoffset` below). Printing "used" beside a
   * "remaining" arc would put two readings of one gauge side by side.
   */
  showRemainingPercent?: boolean;
}

/** Below this share of the window left, the footer prints the figure. */
export const CONTEXT_FIGURE_BELOW_PCT = 50;

/** The ring: 14px (the footer's chip size), a 2px stroke on a 16-unit box. */
const RING_RADIUS = 6;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

/** The footer's context readout: a ring that opens the gauge (threshold and
 * Compact) in a popover. One line of tooltip, "72% context left · 92k of 128k". */
export const ContextWindowIndicator: React.FC<ContextWindowIndicatorProps> = ({
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  onCompact,
  showRemainingPercent = false,
}) => {
  const [open, setOpen] = useState(false);
  const current = totalTokens ?? 0;
  if (!isTokenLimitLoaded && !current) return null;
  const total = tokenLimit || 0;
  // NO MODEL, NO GAUGE. A context window is a property of a bound model, so
  // with none there is no number to report: it once read "128k of 128k tokens
  // remaining" during onboarding beside a chip that said no model was chosen.
  // Asserted on the LIMIT rather than only on the loaded flag, because a lookup
  // that finished and found nothing is exactly the case being guarded.
  if (total <= 0) return null;
  const remainingTokens = Math.max(total - current, 0);
  const remainingRatio = Math.max(0, Math.min(1, remainingTokens / total));
  const remainingPct = Math.round(remainingRatio * 100);
  const strokeOffset = RING_CIRCUMFERENCE * (1 - remainingRatio);
  const summary = FOOTER_COPY.contextTooltip(remainingPct, fmt(current), fmt(total));
  const showFigure = showRemainingPercent && remainingPct < CONTEXT_FIGURE_BELOW_PCT;
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              type="button"
              aria-label={summary}
              data-testid="context-window-indicator"
              className="br-footline__item"
            >
              <svg viewBox="0 0 16 16" aria-hidden="true">
                <circle
                  cx="8"
                  cy="8"
                  r={RING_RADIUS}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  className="text-border-subtle"
                />
                <circle
                  cx="8"
                  cy="8"
                  r={RING_RADIUS}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeDasharray={RING_CIRCUMFERENCE}
                  strokeDashoffset={strokeOffset}
                  className="transition-[stroke-dashoffset] duration-[var(--dur-med-min)]"
                  transform="rotate(-90 8 8)"
                />
              </svg>
              {/* Sans with tabular figures (principle 3: mono is for machine
                  strings only), so a figure that changes as tokens stream does
                  not re-measure the line. Hidden from the accessibility tree:
                  the button's name already says it with its subject. */}
              {showFigure && (
                <span aria-hidden="true">{FOOTER_COPY.contextLeft(remainingPct)}</span>
              )}
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="top">{summary}</TooltipContent>
      </Tooltip>
      <PopoverContent side="top" align="end" className="w-72 p-1">
        <ContextWindowGauge
          totalTokens={totalTokens}
          tokenLimit={tokenLimit}
          isTokenLimitLoaded={isTokenLimitLoaded}
          onCompact={() => {
            onCompact();
            setOpen(false);
          }}
        />
      </PopoverContent>
    </Popover>
  );
};
