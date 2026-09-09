import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronsDownUp } from './icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
import { Progress } from './ui/progress';
import { useConfig } from './ConfigContext';

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
    <div className="flex items-center gap-2 rounded-element px-2 py-1.5">
      <span className="flex size-icon-row flex-shrink-0 items-center justify-center text-text-muted">
        <span className="h-3.5 w-3.5 rounded-full border-2 border-current" />
      </span>
      {/* ONE metadata size in this popover. It ran four — 11px here, 13px on the
          numbers below, 11px in the threshold tooltip, 12px elsewhere — for text
          that is all the same kind of thing: a reading off a gauge. They are all
          `supporting` now. And the ink is the semantic muted token rather than
          `text-text-default/60`, which was a second, slightly different muted
          that no family could re-point. */}
      <span className="w-12 flex-shrink-0 text-supporting text-text-muted">Context</span>
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
            label="Context window used"
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
                aria-label="Auto-compact threshold"
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
            <TooltipContent side="top" className="text-supporting">
              Drag to adjust auto-compact threshold ({thresholdPct}%)
            </TooltipContent>
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
              aria-label={current === 0 ? 'Nothing to compact yet' : 'Compact chat'}
              className={`flex size-control-sm items-center justify-center rounded-element transition-colors ${current === 0 ? 'cursor-not-allowed text-text-muted opacity-50' : 'tint-interactive cursor-pointer text-text-muted hover:text-text-default'}`}
            >
              <ChevronsDownUp data-testid="compact-conversation-icon" className="size-4" />
            </button>
          </span>
        </TooltipTrigger>
        <TooltipContent side="top">
          {current === 0 ? 'Nothing to compact yet' : 'Compact chat'}
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
  /** Override the popover trigger tooltip text. */
  triggerTitle?: string;
  /**
   * Print the headroom beside the ring instead of leaving the ring to carry it
   * alone. Opt-in, because a ring on its own is the compact form and the
   * composer's control bar is the only place with the width to spell it out.
   *
   * ⚠ The number is the REMAINING percentage, not the used one, because the arc
   * the ring draws is the remaining arc (`strokeDashoffset` below). Printing
   * "used" next to a "remaining" arc would put two readings of the same gauge
   * side by side, disagreeing, at every value except 50%.
   */
  showRemainingPercent?: boolean;
}

/** Compact-row variant: a single remaining-context ring button which, on click,
 * opens a popover that renders the same bar-style gauge used in the
 * picker. For the chat tab the user sees one compact ring and,
 * on click, gets the full real-time gauge with a Compact button — the
 * same UI the picker exposes. Ring color reflects remaining headroom. */
export const ContextWindowIndicator: React.FC<ContextWindowIndicatorProps> = ({
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  onCompact,
  triggerTitle = 'Context window usage',
  showRemainingPercent = false,
}) => {
  const [open, setOpen] = useState(false);
  const current = totalTokens ?? 0;
  if (!isTokenLimitLoaded && !current) return null;
  const total = tokenLimit || 0;
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
  const remainingTokens = Math.max(total - current, 0);
  const remainingRatio = Math.max(0, Math.min(1, remainingTokens / total));
  const remainingPct = Math.round(remainingRatio * 100);
  const radius = 8;
  const circumference = 2 * Math.PI * radius;
  const strokeOffset = circumference * (1 - remainingRatio);
  const remainingSummary = `${fmt(remainingTokens)} of ${fmt(total)} tokens remaining`;
  const usageSummary = `${remainingPct}% remaining, ${pct}% used`;
  const tooltip = `${triggerTitle}. ${remainingSummary}. ${usageSummary}`;
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            {/* The composer toolbar's two named iconography violations, both
                here: a glyph at 18px on a row where every other glyph is 16
                (§3.8b forbids two sizes in one cluster), inside a 24px-wide box
                that left ~3px of horizontal padding — a ~20px hit zone on a
                control the user is meant to click. It is now the row's 16px
                icon in a 28px box with 6px of padding either side, so the
                pointer target matches what the eye sees. */}
            <button
              type="button"
              aria-label={tooltip}
              className="relative flex h-control-sm flex-shrink-0 cursor-pointer items-center justify-center rounded-element px-1.5 text-text-muted tint-interactive transition-colors hover:text-text-default"
            >
              <svg className="size-icon-row" viewBox="0 0 24 24" aria-hidden="true">
                <circle
                  cx="12"
                  cy="12"
                  r={radius}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.75"
                  className="text-border-subtle"
                />
                <circle
                  cx="12"
                  cy="12"
                  r={radius}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.75"
                  strokeLinecap="round"
                  strokeDasharray={circumference}
                  strokeDashoffset={strokeOffset}
                  className="text-text-muted transition-[stroke-dashoffset] duration-[var(--dur-med-min)]"
                  transform="rotate(-90 12 12)"
                />
              </svg>
              {/* Numerals in mono, like every other figure in the control bar
                  (the cost, the token counts). A proportional font re-measures
                  the label on every token that streams in, and a number that
                  twitches beside a ring that is also moving reads as a glitch
                  rather than as progress. Hidden from the accessibility tree —
                  `aria-label` above already states the same figure in words, and
                  a bare "72%" would only repeat it without its subject. */}
              {/* `text-supporting`, the composer rails' role — see the note in
                  ChatInput.tsx, "THE RAILS' TYPE". This was `text-xs`, which is
                  the same 12px by coincidence rather than by role: a raw
                  Tailwind step here would not have followed the rails when they
                  moved, and its 16px line height is Tailwind's, not the
                  scale's. */}
              {showRemainingPercent && (
                <span aria-hidden="true" className="ml-1.5 font-mono text-supporting tabular-nums">
                  {remainingPct}%
                </span>
              )}
              <span className="sr-only">{tooltip}</span>
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="top" className="w-52 text-left text-pretty">
          <span className="block font-medium">{triggerTitle}</span>
          <span className="block">{remainingSummary}</span>
          <span className="block">{usageSummary}</span>
        </TooltipContent>
      </Tooltip>
      <PopoverContent side="top" align="start" className="w-72 p-1">
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
