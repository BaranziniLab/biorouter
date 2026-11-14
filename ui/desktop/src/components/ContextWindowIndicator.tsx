import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Progress } from './ui/progress';
import { useConfig } from './ConfigContext';
import { CONTEXT_CARD_COPY, FOOTER_COPY } from './bottom_menu/copy';
import {
  FooterCard,
  FooterCardAction,
  FooterCardActions,
  FooterCardDetail,
  FooterCardNote,
  FooterCardRoot,
  FooterCardSection,
  FooterCardTrigger,
  FooterCardValue,
} from './bottom_menu/FooterCard';
import {
  TOOL_COUNT_COPY,
  toolCountWarning,
  type ToolCountWarning,
} from './alerts/toolCountWarning';
import { useFooterCard } from './bottom_menu/useFooterCard';
import { InfoTip } from './ui/info-tip';
import {
  AUTO_COMPACT_MAX_TOKENS_KEY,
  effectiveAutoCompactTrigger,
  savedAutoCompactCeiling,
} from '../utils/autoCompactTrigger';
import './bottom_menu/pickers.css';

interface ContextWindowGaugeProps {
  /**
   * The chat's tool count once it is past what works well (`alerts/toolCountWarning.ts`): one
   * line with an InfoTip, and "Review extensions" beside Compact chat. Absent below the limit.
   */
  tools?: ToolCountWarning | null;
  totalTokens: number | undefined;
  tokenLimit: number;
  isTokenLimitLoaded: boolean;
  onCompact: () => void;
}

const AUTO_COMPACT_THRESHOLD_KEY = 'BIOROUTER_AUTO_COMPACT_THRESHOLD';
const AUTO_COMPACT_MIN_PCT = 20;
const AUTO_COMPACT_MAX_PCT = 90;
const AUTO_COMPACT_DEFAULT_PCT = 80;

/**
 * The note while the token ceiling, not the marker's share, is where Biorouter compacts (a window
 * over 250k tokens by default). Local rather than in `CONTEXT_CARD_COPY` because only this card
 * says it. One line in the card; why the marker does not move it is the note's InfoTip (owner,
 * 2026-11-13: one line up front, the detail on hover).
 */
export const CEILING_NOTE = (tokens: string, pct: number) =>
  `Compacts automatically at ${tokens} tokens used (${pct}%).`;
export const CEILING_HELP_LABEL = 'the compaction limit';
export const CEILING_HELP =
  'That is the limit for any window. The marker sets a share of the window, which applies only below it.';
/** The ceiling tick's accessible description, read with the bar. */
export const CEILING_TICK_LABEL = (tokens: string) => `Compacts at ${tokens} tokens`;

function clampThresholdPct(v: number): number {
  if (!Number.isFinite(v)) return AUTO_COMPACT_DEFAULT_PCT;
  return Math.max(AUTO_COMPACT_MIN_PCT, Math.min(AUTO_COMPACT_MAX_PCT, Math.round(v)));
}

/**
 * What the saved threshold makes Biorouter do, read the way the daemon reads it
 * (`check_if_compaction_needed` in crates/biorouter/src/context_mgmt/mod.rs): a
 * share strictly between 0 and 1 is where it compacts, and anything at or
 * outside those bounds turns automatic compaction off (0 is the documented
 * "disabled"). A value that is not a number leaves the daemon's default, so it
 * reads as nothing here either. The percentage is the real one, not the
 * marker's: the marker only reaches 20 to 90%, and a sentence that said "90%"
 * for a saved 0.95 would be wrong about what happens.
 */
function savedAutoCompact(v: unknown): 'off' | number | null {
  if (typeof v !== 'number' || !Number.isFinite(v)) return null;
  if (v <= 0 || v >= 1) return 'off';
  // Never "0%" or "100%" for a share strictly between them.
  return Math.max(1, Math.min(99, Math.round(v * 100)));
}

/**
 * The share of the window that is left, from 0 to 1: the arc the ring draws and the figure the
 * card reads, computed once so the two can never round apart. No window reads as all of it left.
 */
function remainingShare(used: number, total: number): number {
  if (total <= 0) return 1;
  return Math.max(0, Math.min(1, Math.max(total - used, 0) / total));
}

/**
 * The context card's body (`ContextWindowIndicator`'s `FooterCard`), built only from the footer
 * cards' pieces so it reads like the folder and cost cards beside it: one "Context window"
 * section holding what is LEFT (as the ring draws it), what is USED (as the bar fills), the bar
 * with its auto-compact marker, the sentence that says where the marker sits, and Compact chat.
 *
 * The bar alone carries colour: green, then yellow past half, then red past the threshold. The
 * marker is the point at which Biorouter auto-compacts, a slider clamped to 20 to 90%: drag it (or
 * press anywhere on the bar), or use the arrow keys, Home and End. It has no hover tooltip: its
 * reading is its `aria-valuetext` and the note under the bar, which says it to everyone. Both say
 * what the saved value makes Biorouter do (`savedAutoCompact`): that automatic compaction is off,
 * or a saved share the marker cannot reach, until the person moves the marker.
 */
export const ContextWindowGauge: React.FC<ContextWindowGaugeProps> = ({
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  onCompact,
  tools,
}) => {
  const current = totalTokens ?? 0;
  const total = tokenLimit || 0;
  // The threshold is read and written through ConfigContext, never straight to
  // the API: the context's cached config is the copy the rest of the app reads,
  // and only a write that goes through it re-reads that cache afterwards. A
  // direct `upsertConfig` here would leave every other consumer of this
  // (non-secret, therefore cached) key on the pre-write value (#52).
  const { read: readConfigValue, upsert: upsertConfigValue } = useConfig();
  /** Where the marker sits: always inside the slider's 20 to 90%. */
  const [thresholdPct, setThresholdPct] = useState<number>(AUTO_COMPACT_DEFAULT_PCT);
  /**
   * What the saved value makes Biorouter do when the marker cannot say it:
   * `'off'`, or a percentage outside the marker's range. `null` once the marker
   * is the threshold (the default, a saved value inside the range, or any
   * change the person makes, which always writes the marker's value).
   */
  const [savedOutsideMarker, setSavedOutsideMarker] = useState<'off' | number | null>(null);
  /** The absolute token ceiling (`BIOROUTER_AUTO_COMPACT_MAX_TOKENS`), `null` for none. */
  const [ceiling, setCeiling] = useState<number | null>(savedAutoCompactCeiling(undefined));
  const nothingToCompactId = useId();
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
        const saved = savedAutoCompact(v);
        if (saved === null) return;
        if (saved === 'off') {
          // The marker stays where a change would start from, the default.
          setSavedOutsideMarker('off');
          return;
        }
        const marker = clampThresholdPct(saved);
        setThresholdPct(marker);
        setSavedOutsideMarker(marker === saved ? null : saved);
      })
      .catch(() => {
        /* fall back to default */
      });
    return () => {
      cancelled = true;
    };
  }, [readConfigValue]);

  // The ceiling is read once, like the threshold; it is set in config, not here.
  useEffect(() => {
    let cancelled = false;
    readConfigValue(AUTO_COMPACT_MAX_TOKENS_KEY, false)
      .then((v) => {
        if (!cancelled) setCeiling(savedAutoCompactCeiling(v));
      })
      .catch(() => {
        /* the daemon's default stands */
      });
    return () => {
      cancelled = true;
    };
  }, [readConfigValue]);

  // Flush any in-flight pending change if the gauge unmounts mid-drag
  // (e.g. the card closes when the person releases the mouse outside of it).
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
  // Every change the person makes writes the marker's value, which turns
  // automatic compaction on at that share and makes the marker the threshold.
  const updateThresholdLive = (raw: number) => {
    const next = clampThresholdPct(raw);
    setThresholdPct(next);
    setSavedOutsideMarker(null);
    pendingPctRef.current = next;
  };

  const handleSliderChange = (raw: number) => {
    const next = clampThresholdPct(raw);
    setThresholdPct(next);
    setSavedOutsideMarker(null);
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
      // NOT `preventDefault()`: the press's own focus is what keeps the card.
      // A press inside any footer card moves focus into it (onto the marker,
      // which is focusable, or onto the card's surface for a press on the bar),
      // and only a card that holds focus outlives a window switch: coming back,
      // the window refocuses whatever had focus, and a draft that had kept it
      // was "focus outside" to Radix, which closed the card the press had
      // pinned. The browser's focus for a press draws no focus edge; a script
      // focus here would, since it inherits the draft's. What the cancelled
      // default also stopped, a text selection dragged out of the bar, is
      // `user-select: none` on `.br-context-gauge` (pickers.css).
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

  // A window that has not loaded is the composer's 128k fallback, not the
  // model's: a reading against it is wrong (V2-R4-1), so the gauge draws none.
  if (!isTokenLimitLoaded) return null;
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
  const leftPct = Math.round(remainingShare(current, total) * 100);
  // What Biorouter will do, which the marker can only show between 20 and 90%:
  // `null` when automatic compaction is off.
  const compactsAt = savedOutsideMarker === 'off' ? null : (savedOutsideMarker ?? thresholdPct);
  // Where it really fires: the lower of that share and the token ceiling.
  const trigger = effectiveAutoCompactTrigger(compactsAt, total, ceiling);
  const ceilingTick = trigger?.byCeiling ? trigger : null;
  const overThreshold = ceilingTick ? ratio * 100 >= ceilingTick.pct : pct >= thresholdPct;
  // Three bands, not four: the old ladder spelled `warning` twice (`ratio <= 0.75`
  // and the fallback both resolved to it), so the 0.75 rung was a no-op.
  const barTone = overThreshold ? 'danger' : ratio <= 0.5 ? 'success' : 'warning';
  const empty = current === 0;
  return (
    <FooterCardSection label={CONTEXT_CARD_COPY.section}>
      {/* What is LEFT is the reading, because it is what the ring beside it
          draws; what is USED is the detail, because it is what the bar fills. */}
      <FooterCardValue>{CONTEXT_CARD_COPY.left(leftPct)}</FooterCardValue>
      <FooterCardDetail>{CONTEXT_CARD_COPY.used(fmt(current), fmt(total))}</FooterCardDetail>
      {/* The bar holds the usage fill *and* a draggable downward triangle
          marking the auto-compact threshold. The bar is the drag target;
          mousedown anywhere on the bar (or the triangle) jumps the threshold
          to the cursor and starts a drag. */}
      {/* The threshold triangle is a SIBLING of the bar, not a child of it.
          It used to live inside the track, which forced the track to run
          `overflow-visible` so the glyph could stick out above — the one
          reason this bar could not be the shared `Progress` (an 8px pill that
          clips its fill). Both are absolutely positioned against this same
          wrapper and the wrapper is exactly as wide as the track, so
          `left: {pct}%` means the identical thing it did before; only the
          vertical origin moved, from the track's top edge to the wrapper's
          (`.br-context-gauge*` in pickers.css). */}
      <div className="br-context-gauge">
        <Progress
          ref={barRef}
          label={CONTEXT_CARD_COPY.barLabel}
          value={pct}
          tone={barTone}
          // A conversation with one message is still using context.
          minVisiblePercent={2}
          className="br-context-gauge__bar"
          onMouseDown={handleDragStart}
        />
        {/* Hit area larger than the 10×6 visible triangle, so a press that
            grazes the glyph still lands on it. The visible glyph is the inner
            element. */}
        <div
          role="slider"
          aria-label={CONTEXT_CARD_COPY.thresholdLabel}
          aria-valuemin={AUTO_COMPACT_MIN_PCT}
          aria-valuemax={AUTO_COMPACT_MAX_PCT}
          aria-valuenow={thresholdPct}
          aria-valuetext={
            compactsAt === null
              ? CONTEXT_CARD_COPY.thresholdOff
              : CONTEXT_CARD_COPY.thresholdValue(compactsAt)
          }
          tabIndex={0}
          onKeyDown={handleKeyAdjust}
          onMouseDown={handleDragStart}
          className="br-context-gauge__marker"
          style={{ left: `${thresholdPct}%` }}
        >
          <span aria-hidden="true" className="br-context-gauge__glyph" />
        </div>
        {/* Where the token ceiling makes Biorouter compact, when that comes
            before the marker's share. Not a control: the ceiling is set in
            config. Drawn as a filled tick in the bar's own ink, so it reads as
            the point the bar runs into rather than a second handle. */}
        {ceilingTick && (
          <span
            role="img"
            aria-label={CEILING_TICK_LABEL(fmt(ceilingTick.tokens))}
            data-testid="context-gauge-ceiling"
            className="br-context-gauge__ceiling"
            style={{
              position: 'absolute',
              left: `${Math.max(0, Math.min(100, ceilingTick.pct))}%`,
              bottom: 0,
              width: 2,
              height: 14,
              transform: 'translateX(-50%)',
              background: 'var(--text-default)',
              borderRadius: 1,
              pointerEvents: 'none',
            }}
          />
        )}
      </div>
      <FooterCardNote>
        {compactsAt === null ? (
          CONTEXT_CARD_COPY.autoCompactOff
        ) : ceilingTick ? (
          <>
            {CEILING_NOTE(fmt(ceilingTick.tokens), Math.max(1, Math.round(ceilingTick.pct)))}
            <InfoTip label={CEILING_HELP_LABEL} help={CEILING_HELP} />
          </>
        ) : (
          CONTEXT_CARD_COPY.autoCompactNote(compactsAt)
        )}
      </FooterCardNote>
      {tools && (
        <FooterCardNote>
          <span data-testid="context-card-tool-count">{TOOL_COUNT_COPY.line(tools.count)}</span>
          <InfoTip label={TOOL_COUNT_COPY.helpLabel} help={TOOL_COUNT_COPY.help} />
        </FooterCardNote>
      )}
      <FooterCardActions>
        <FooterCardAction
          disabled={empty}
          aria-describedby={empty ? nothingToCompactId : undefined}
          onClick={() => onCompact()}
        >
          {CONTEXT_CARD_COPY.compact}
        </FooterCardAction>
        {tools && (
          <FooterCardAction onClick={tools.review} data-testid="context-card-review-extensions">
            {TOOL_COUNT_COPY.review}
          </FooterCardAction>
        )}
      </FooterCardActions>
      {empty && (
        <FooterCardNote id={nothingToCompactId}>
          {CONTEXT_CARD_COPY.nothingToCompact}
        </FooterCardNote>
      )}
    </FooterCardSection>
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
   * The chat this ring reports on. A change of chat is never a live update: the
   * ring shows the new chat's value at rest instead of sweeping to it.
   */
  sessionId?: string | null;
  /**
   * The model the ring must measure for NOW, spelled by
   * {@link contextWindowModelKey}: the chat's own binding, else the app's
   * selection. `null` while that is not known yet: the chat is loading, so its
   * row, and the model the row names, has not landed, and the composer meanwhile
   * looked up the window of the model for NEW chats (V2-R4-1, round 5).
   *
   * Omitted, a loaded window is taken to be this chat's (a caller with no
   * binding to wait for).
   */
  boundModel?: string | null;
  /** The model `tokenLimit` was looked up for, spelled like `boundModel`. */
  tokenLimitModel?: string | null;
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
  /**
   * The tools this chat can call (`useToolCount`). Past `TOOLS_MAX_SUGGESTED` the card says so
   * and offers "Review extensions", which opens the composer's `+` on this chat's Extensions.
   */
  toolCount?: number | null;
}

/** Below this share of the window left, the footer prints the figure. */
export const CONTEXT_FIGURE_BELOW_PCT = 50;

/** The ring: 14px (the footer's chip size), a 2px stroke on a 16-unit box. */
const RING_RADIUS = 6;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

/**
 * Whether the arc lands at rest in this render (spec 2.6: a value that loads
 * asynchronously does not animate; checklist 12: nothing replays on a tab switch).
 *
 * The arc sweeps (`.br-footline__ring-arc`, 250ms) only for a LIVE update: a new
 * value in the same chat, where the render before it already showed a loaded
 * reading (a known window and a token count). Everything else snaps: the chat's
 * first reading after it opens or its tab is switched to (the count and the
 * window both arrive about a second after the composer mounts, and the arc used
 * to sweep from the empty ring to them), a change of chat under the same
 * composer, and the window re-read after a model change.
 *
 * Derived DURING the render that moves the arc, never in an effect after it: the
 * Switch's rule (V4-01), because a transition the browser has started ignores a
 * later `transition-duration: 0ms`. Released two frames later, so at least one
 * frame is styled with the flag.
 */
function useArcStill(
  sessionId: string | null | undefined,
  offset: number | null,
  loaded: boolean
): boolean {
  const [shown, setShown] = useState({ sessionId, offset, loaded });
  const [still, setStill] = useState(0);

  if (shown.sessionId !== sessionId || shown.offset !== offset || shown.loaded !== loaded) {
    setShown({ sessionId, offset, loaded });
    if (shown.offset !== offset) {
      const live =
        shown.sessionId === sessionId &&
        shown.loaded &&
        loaded &&
        shown.offset !== null &&
        offset !== null;
      // A fresh token per change restarts the release.
      setStill(live ? 0 : still + 1);
    }
  }

  useEffect(() => {
    if (!still) return;
    let second = 0;
    const first = window.requestAnimationFrame(() => {
      second = window.requestAnimationFrame(() => setStill(0));
    });
    return () => {
      window.cancelAnimationFrame(first);
      window.cancelAnimationFrame(second);
    };
  }, [still]);

  return still > 0;
}

/**
 * How the composer spells "the model the context window belongs to" for
 * `boundModel` and `tokenLimitModel`: the chat's binding when it has one, else
 * the app's selection. One spelling in one place, so the two props can only be
 * compared, never mis-spelled apart.
 */
export function contextWindowModelKey(
  binding: { provider: string; model: string } | undefined,
  selectedProvider: string | null | undefined,
  selectedModel: string | null | undefined
): string {
  return binding
    ? `${binding.provider}/${binding.model}`
    : `${selectedProvider ?? ''}/${selectedModel ?? ''}`;
}

interface DrawnWindow {
  /** The window to draw against, or 0 for none. */
  limit: number;
  /** A lookup that landed for this chat's model, not a window held across a re-read. */
  fresh: boolean;
}

/**
 * The window the ring may draw against: one the composer LOADED for this chat's
 * model, or none (V2-R4-1).
 *
 * The composer mounts with a 128k fallback in `tokenLimit` and `isTokenLimitLoaded`
 * false, and the chat's real window lands about a second later. The ring used to
 * draw at once whenever the chat had tokens, so every open and tab switch showed a
 * reading against a window nobody had looked up: "20% left" on a 1M chat that was
 * 10% full, and on a chat past 128k the empty ring and "0% left". So nothing is
 * drawn until a window has loaded.
 *
 * LOADED IS NOT ENOUGH: IT MUST BE THIS CHAT'S MODEL (round 5). A chat opened for
 * the first time (a tab restored after a reload, a hash or History open) loads
 * for up to seconds, and until its row lands the composer has no binding and
 * looks up the window of the model for new chats. That lookup is real and
 * loaded, for the wrong model: a 200k chat read "0 of 1.1M", then swept to
 * "64.6k of 1.1M" and snapped to "of 200k" five seconds later. So with
 * `boundModel` given, a window draws only when it was looked up for exactly that
 * model, and nothing draws while `boundModel` is `null` (the binding unknown).
 *
 * A RE-READ OF THE SAME MODEL (the app's selection moved under a chat pinned to
 * its own model, which re-runs the lookup with the same answer) holds the last
 * window that model loaded rather than blinking the ring out for a second. A
 * window held for one model never stands in for another: after a model change
 * the ring draws nothing until the new model's window lands. The hold is per
 * chat and ends with the mount, so a tab switch (BaseChat remounts per tab)
 * never carries one over, and a re-read that finds no model (`tokenLimit` 0)
 * draws nothing.
 *
 * Derived during render, not in an effect, so the frame that would show a wrong
 * window is never painted.
 */
function useDrawnWindow(
  sessionId: string | null | undefined,
  boundModel: string | null | undefined,
  tokenLimitModel: string | null | undefined,
  tokenLimit: number,
  loaded: boolean
): DrawnWindow {
  const [held, setHeld] = useState<{
    sessionId: string | null | undefined;
    model: string | null | undefined;
    limit: number;
  } | null>(null);
  // Omitted (`undefined`): no binding to wait for, a loaded window is this chat's.
  const forThisModel = boundModel === undefined || tokenLimitModel === boundModel;
  const fresh = loaded && tokenLimit > 0 && boundModel !== null && forThisModel;
  if (
    fresh &&
    (!held ||
      held.sessionId !== sessionId ||
      held.model !== boundModel ||
      held.limit !== tokenLimit)
  ) {
    setHeld({ sessionId, model: boundModel, limit: tokenLimit });
  }
  if (fresh) return { limit: tokenLimit, fresh: true };
  if (tokenLimit <= 0 || boundModel === null) return { limit: 0, fresh: false };
  const holds = held !== null && held.sessionId === sessionId && held.model === boundModel;
  return { limit: holds ? held.limit : 0, fresh: false };
}

/**
 * The footer's context readout (spec 3.7), one of the line's three items that behave alike
 * (`useFooterCard`, `FooterCard`): a ring that draws what is left of the window, "N% left" beside
 * it once less than half is left, and one card. Hovering the ring (300ms), reaching it with Tab or
 * clicking it opens the card; a click pins it, a second click closes it, and Enter or Space also
 * moves focus into it. The card is `ContextWindowGauge`: what is left and used, the bar with the
 * auto-compact marker, and Compact chat, which runs the compaction and closes the card.
 *
 * It replaced a dark one-line tooltip on hover and a separate popover on click, so the ring now
 * answers a hover, a click and a key the same way the folder chip and the cost beside it do. The
 * ring's name still says the whole reading ("72% context left · 92k of 128k"), so a screen reader
 * has it without opening the card.
 */
export const ContextWindowIndicator: React.FC<ContextWindowIndicatorProps> = ({
  sessionId,
  boundModel,
  tokenLimitModel,
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  onCompact,
  showRemainingPercent = false,
  toolCount,
}) => {
  const card = useFooterCard();
  const overTools = toolCountWarning(toolCount, sessionId);
  // The card closes first, then the `+` popover opens on Extensions in the next frame, so the
  // card's focus return to the ring never lands after the popover took focus for its search.
  const tools: ToolCountWarning | null = overTools && {
    count: overTools.count,
    review: () => {
      card.close();
      requestAnimationFrame(() => overTools.review());
    },
  };
  const cardId = useId();
  const current = totalTokens ?? 0;
  // NO WINDOW LOADED FOR THIS CHAT'S MODEL, NO RING (V2-R4-1). A context window
  // is a property of a bound model, and the composer's 128k fallback stands in
  // for it until the lookup lands: drawn against it, a 1M chat read "20% left"
  // for a second on every tab switch; drawn against the new-chat model's window
  // while a chat loads, a 200k chat read "of 1.1M". With no model there is no
  // window at all: the ring once read "128k of 128k tokens remaining" during
  // onboarding beside a chip that said no model was chosen. Asserted on the
  // LIMIT as well as the loaded flag, because a lookup that finished and found
  // nothing is exactly that case.
  const drawnWindow = useDrawnWindow(
    sessionId,
    boundModel,
    tokenLimitModel,
    tokenLimit || 0,
    isTokenLimitLoaded
  );
  const total = drawnWindow.limit;
  const drawn = total > 0;
  const remainingRatio = remainingShare(current, total);
  const remainingPct = Math.round(remainingRatio * 100);
  const strokeOffset = RING_CIRCUMFERENCE * (1 - remainingRatio);
  // A reading is "loaded" for the sweep only when its window is a fresh lookup
  // for this chat's model: the first reading after a chat loads has nothing
  // drawn before it, so it lands at rest rather than sweeping from the empty ring.
  const arcStill = useArcStill(
    sessionId,
    drawn ? strokeOffset : null,
    drawnWindow.fresh && totalTokens !== undefined
  );
  // A ring that stops being drawn (a model change, a chat that loads again)
  // takes its card with it, and the card does not come back open by itself
  // when the ring returns: nothing the person did then opened it.
  const closeCard = card.close;
  useEffect(() => {
    if (!drawn) closeCard();
  }, [drawn, closeCard]);
  if (!drawn) return null;
  const summary = FOOTER_COPY.contextTooltip(remainingPct, fmt(current), fmt(total));
  const showFigure = showRemainingPercent && remainingPct < CONTEXT_FIGURE_BELOW_PCT;
  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger
        card={card}
        cardId={cardId}
        aria-label={summary}
        data-testid="context-window-indicator"
      >
        <svg viewBox="0 0 16 16" aria-hidden="true">
          {/* The track: authored ink (`.br-footline__ring-track`, pickers.css), which a kept
              trigger deepens so the circle still shows on its strong fill. */}
          <circle
            cx="8"
            cy="8"
            r={RING_RADIUS}
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            className="br-footline__ring-track"
            data-testid="context-ring-track"
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
            className="br-footline__ring-arc"
            data-testid="context-ring-arc"
            data-motion-still={arcStill ? '' : undefined}
            transform="rotate(-90 8 8)"
          />
        </svg>
        {/* Sans with tabular figures (principle 3: mono is for machine
            strings only), so a figure that changes as tokens stream does
            not re-measure the line. Hidden from the accessibility tree:
            the button's name already says it with its subject. */}
        {showFigure && <span aria-hidden="true">{FOOTER_COPY.contextLeft(remainingPct)}</span>}
      </FooterCardTrigger>
      <FooterCard
        card={card}
        id={cardId}
        label={CONTEXT_CARD_COPY.cardLabel}
        align="end"
        testId="context-card"
      >
        {/* The card reads the window the ring draws (loaded, or held across a
            re-read), so the two never disagree. */}
        <ContextWindowGauge
          totalTokens={totalTokens}
          tokenLimit={total}
          isTokenLimitLoaded
          onCompact={() => {
            onCompact();
            card.close();
          }}
          tools={tools}
        />
      </FooterCard>
    </FooterCardRoot>
  );
};
