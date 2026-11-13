/**
 * The composer's microphone (DD-60 to DD-62, "The desktop composer button in
 * every composer" in `docs/dictation/surfaces.md`), and the notice row it
 * reports into.
 *
 * A ghost round 28px button left of the composer's send action. Its phase is
 * `data-dictation-phase`, styled in authored CSS (`composer/composer.css`),
 * because a newly written Tailwind class may never generate under
 * `BIOROUTER_NO_HMR`. While recording it becomes a 28px pill with five level
 * bars and the `m:ss` timer (the timer is dropped in a pane narrower than
 * 440px). The bars are driven from the session's level in an animation frame
 * through a CSS custom property, never through React, and stand still under
 * `prefers-reduced-motion`.
 *
 * It starts and stops on `pointerdown`, never on `click`: the person's press
 * to release time (often 50 to 120 ms) is then off both the press path and the
 * stop path, and `pointerdown` is still a user activation for
 * `getUserMedia`. `mousedown` is cancelled, so the field keeps its focus and its
 * accent edge. A click from the keyboard (Enter or Space on the focused button,
 * which has no pointer down) presses it the same way.
 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { cn } from '../../utils';
import { isBrowserSurface } from '../../utils/surface';
import {
  getDictationAriaKeyShortcuts,
  getDictationShortcutText,
} from '../../utils/keyboardShortcuts';
import type { View, ViewOptions } from '../../utils/navigationUtils';
import { Mic } from '../icons/app-icons';
import { CHAT_COLUMN_CHROME } from '../Layout/yieldLadder';
import { Button } from '../ui/button';
import { Popover, PopoverAnchor, PopoverContent } from '../ui/popover';
import { Spinner } from '../ui/spinner';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import {
  DICTATION_ACTIONS,
  DICTATION_LABELS,
  dictationMessage,
  fillMessage,
  formatDictationClock,
} from './copy';
import {
  dismissDictationNotice,
  getDictationLevel,
  getDictationSnapshot,
  retryDictation,
  subscribeDictation,
  type DictationActivity,
  type DictationSnapshot,
} from './dictationSession';
import {
  readDictationViewer,
  resolveDictationPhase,
  useDictationStatus,
  type PhaseDecision,
} from './dictationStore';
import { SetupPopover } from './SetupPopover';
import { SharedOriginNotice } from './SharedOriginNotice';
import type { DictationComposerControl } from './useDictationComposer';

export type DictationPhase =
  | 'unavailable'
  | 'not-installed'
  | 'installing'
  | 'idle'
  | 'arming'
  | 'recording'
  | 'transcribing'
  | 'error';

/** The pane width under which the recording pill drops its timer. */
export const DICTATION_TIMER_MIN_PANE_PX = 440;
/** Level bars in the recording pill. */
export const DICTATION_LEVEL_BARS = 5;

/**
 * A chunk's RMS on the meter's 0 to 1 scale: -60 dBFS (the noise floor the
 * session's MSG-NO-AUDIO-YET uses) to full scale.
 */
export function levelToMeter(rms: number): number {
  if (!(rms > 0)) return 0;
  const db = 20 * Math.log10(rms);
  return Math.max(0, Math.min(1, (db + 60) / 60));
}

/** What the microphone shows: the session's activity for this composer first, then the status. */
export function dictationPhaseFor(
  activity: DictationActivity,
  decision: PhaseDecision
): DictationPhase {
  if (activity === 'arming' || activity === 'recording' || activity === 'transcribing') {
    return activity;
  }
  if (decision.phase === 'unavailable') return 'unavailable';
  if (decision.phase === 'not-installed' || decision.phase === 'installing') return decision.phase;
  return activity === 'error' ? 'error' : 'idle';
}

/** The session's snapshot; it is a cached object, so it also serves a server render. */
function useSnapshot(): DictationSnapshot {
  return useSyncExternalStore(subscribeDictation, getDictationSnapshot, getDictationSnapshot);
}

const reducedMotion = (): boolean => {
  try {
    return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true;
  } catch {
    return false;
  }
};

export interface DictationButtonProps {
  control: DictationComposerControl;
  setView: (view: View, options?: ViewOptions) => void;
  className?: string;
  idleTooltip?: string;
}

export function DictationButton({
  control,
  setView,
  className,
  idleTooltip,
}: DictationButtonProps) {
  const snapshot = useSnapshot();
  const statusSnapshot = useDictationStatus();
  const decision = useMemo(() => resolveDictationPhase(statusSnapshot), [statusSnapshot]);
  const activity: DictationActivity = snapshot.key === control.key ? snapshot.activity : 'idle';
  const phase = dictationPhaseFor(activity, decision);
  const [showButton] = useState(() => readDictationViewer().showButton);
  const [setupOpen, setSetupOpen] = useState(false);
  const [compact, setCompact] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const pillRef = useRef<HTMLSpanElement>(null);

  // A press found no model (or the daemon answered 409 `noModel`): open the popover.
  const setupRequest = snapshot.setupRequest;
  useEffect(() => {
    if (setupRequest?.key === control.key) setSetupOpen(true);
  }, [setupRequest, control.key]);
  // Once the install it was opened for has given a usable model, the popover has
  // done its job. Only that change closes it: a 409 `noModel` opens it while the
  // status still reads `idle`.
  const lastDecisionPhase = useRef(decision.phase);
  useEffect(() => {
    const before = lastDecisionPhase.current;
    lastDecisionPhase.current = decision.phase;
    if ((before === 'not-installed' || before === 'installing') && decision.phase === 'idle') {
      setSetupOpen(false);
    }
  }, [decision.phase]);

  // The level bars: read the session's level in each animation frame and write
  // it into one custom property. No React render per chunk or per frame.
  useEffect(() => {
    if (phase !== 'recording') return;
    const pill = pillRef.current;
    if (!pill || reducedMotion() || typeof requestAnimationFrame !== 'function') return;
    let shown = 0;
    let frame = 0;
    const draw = () => {
      shown += (levelToMeter(getDictationLevel()) - shown) * 0.35;
      pill.style.setProperty('--br-dictation-level', shown.toFixed(3));
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(frame);
      pill.style.removeProperty('--br-dictation-level');
    };
  }, [phase]);

  // The timer is dropped in a pane narrower than 440px, measured on the card
  // while the pill shows, and set only when the answer flips.
  const pill = phase === 'recording' || phase === 'transcribing';
  useEffect(() => {
    if (!pill || typeof ResizeObserver !== 'function') return;
    const card = buttonRef.current?.closest('.biorouter-composer-card');
    if (!card) return;
    const measure = () =>
      setCompact(
        card.getBoundingClientRect().width + CHAT_COLUMN_CHROME < DICTATION_TIMER_MIN_PANE_PX
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(card);
    return () => observer.disconnect();
  }, [pill]);

  if (!control.available) return null;
  // Hidden by the person's choice: the chord still dictates, and the pill takes
  // the button's place until the dictation ends. A chord that found no model
  // shows the button too, for as long as the setup popover it opened, so the
  // press never silently does nothing.
  if (!showButton && !pill && phase !== 'arming' && !setupOpen) return null;

  const browser = isBrowserSurface();
  const chord = browser ? '' : getDictationShortcutText();
  const clock = formatDictationClock(snapshot.key === control.key ? snapshot.elapsedS : 0);
  const label =
    phase === 'unavailable'
      ? (decision.reason ?? '')
      : phase === 'not-installed'
        ? DICTATION_LABELS.setUp
        : phase === 'installing'
          ? fillMessage(DICTATION_LABELS.installing, { n: String(decision.progress ?? 0) })
          : phase === 'arming'
            ? DICTATION_LABELS.arming
            : phase === 'recording'
              ? fillMessage(DICTATION_LABELS.recording, { time: clock })
              : phase === 'transcribing'
                ? DICTATION_LABELS.transcribing
                : DICTATION_LABELS.idle;
  const tooltip =
    phase === 'recording'
      ? DICTATION_LABELS.recordingTooltip
      : phase === 'idle' || phase === 'error'
        ? (idleTooltip ?? (browser ? DICTATION_LABELS.idle : `${DICTATION_LABELS.idle} ${chord}`))
        : label;

  const press = () => {
    if (phase === 'not-installed' || phase === 'installing') {
      setSetupOpen((open) => !open);
      control.noteInterest();
      return;
    }
    if (phase === 'unavailable' || phase === 'transcribing') return;
    control.press();
  };

  const glyph =
    phase === 'installing' ? (
      <span className="br-dictation-ring" aria-hidden>
        <svg viewBox="0 0 28 28" width="28" height="28">
          <circle className="br-dictation-ring__track" cx="14" cy="14" r="12" />
          <circle
            className="br-dictation-ring__value"
            cx="14"
            cy="14"
            r="12"
            pathLength={100}
            strokeDasharray={`${decision.progress ?? 0} 100`}
          />
        </svg>
        <Mic className="size-4" />
      </span>
    ) : phase === 'arming' ? (
      <span className="br-dictation-arming" aria-hidden>
        <Mic className="br-dictation-arming__mic size-4" />
        <span className="br-dictation-arming__spinner">
          <Spinner size={16} />
        </span>
      </span>
    ) : pill ? (
      <span ref={pillRef} className="br-dictation-pill" aria-hidden>
        {phase === 'transcribing' ? (
          <Spinner size={14} />
        ) : (
          <span className="br-dictation-bars">
            {Array.from({ length: DICTATION_LEVEL_BARS }, (_, index) => (
              <span key={index} className="br-dictation-bar" data-bar={index} />
            ))}
          </span>
        )}
        {!compact && (
          <span className="br-dictation-timer" data-testid="dictation-timer">
            {clock}
          </span>
        )}
      </span>
    ) : (
      <Mic className="size-4" aria-hidden />
    );

  const button = (
    <Button
      ref={buttonRef}
      type="button"
      size="sm"
      shape="round"
      variant="ghost"
      data-testid="dictation-button"
      data-dictation-phase={phase}
      disabled={phase === 'unavailable'}
      aria-label={label}
      aria-pressed={phase === 'arming' || phase === 'recording'}
      aria-busy={phase === 'arming' || phase === 'transcribing' || undefined}
      aria-keyshortcuts={browser ? undefined : getDictationAriaKeyShortcuts()}
      aria-haspopup={phase === 'not-installed' || phase === 'installing' ? 'dialog' : undefined}
      aria-expanded={phase === 'not-installed' || phase === 'installing' ? setupOpen : undefined}
      className={cn('br-dictation-button', className)}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        press();
      }}
      // The field keeps its focus, and so its caret and its accent edge.
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => {
        // A pointer press already acted on `pointerdown`; a keyboard click has no pointer.
        if (event.detail === 0) press();
      }}
      onPointerEnter={control.noteInterest}
      onFocus={control.noteInterest}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && control.holdsComposer) {
          event.preventDefault();
          control.cancel();
        }
      }}
    >
      {glyph}
    </Button>
  );

  const withTooltip =
    phase === 'unavailable' ? (
      <Tooltip>
        <TooltipTrigger asChild>
          {/* A disabled button gets no pointer events; the span still opens the tooltip. */}
          <span className="br-dictation-slot" tabIndex={0} aria-label={label}>
            {button}
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-80">{label}</TooltipContent>
      </Tooltip>
    ) : (
      <Tooltip>
        <TooltipTrigger asChild>{button}</TooltipTrigger>
        <TooltipContent>{tooltip}</TooltipContent>
      </Tooltip>
    );

  return (
    <>
      <Popover open={setupOpen} onOpenChange={setSetupOpen}>
        <PopoverAnchor asChild>
          <span className="br-dictation-slot">{withTooltip}</span>
        </PopoverAnchor>
        {setupOpen && (
          <PopoverContent
            side="top"
            align="end"
            className="w-80 p-0"
            onInteractOutside={(event) => {
              // The microphone toggles the popover itself, on its own pointer down.
              if (buttonRef.current?.contains(event.target as Node)) event.preventDefault();
            }}
          >
            <SetupPopover
              status={statusSnapshot.status}
              progress={phase === 'installing' ? (decision.progress ?? 0) : null}
              onNotNow={() => setSetupOpen(false)}
              onMoreInSettings={() => {
                setSetupOpen(false);
                setView('settings', { section: 'dictation' } as ViewOptions);
              }}
            />
          </PopoverContent>
        )}
      </Popover>
      <SharedOriginNotice
        open={control.sharedOriginOpen}
        onCancel={control.cancelSharedOrigin}
        onContinue={control.acknowledgeSharedOrigin}
      />
    </>
  );
}

/**
 * Why Send waits while dictation holds the composer: "Finish dictating first"
 * while arming or recording, "Transcribing…" after the stop. Its own subscriber,
 * so the composer does not render on the stop to change a tooltip.
 */
export function DictationHoldReason({ dictationKey }: { dictationKey: string }) {
  const snapshot = useSnapshot();
  const transcribing = snapshot.key === dictationKey && snapshot.activity === 'transcribing';
  return <>{transcribing ? DICTATION_LABELS.transcribing : DICTATION_LABELS.finishFirst}</>;
}

/**
 * The card's notice row for one composer: MSG-WAIT, the muted partial line, a
 * failure or result with its one action, and a visually hidden `role="status"`
 * that announces Recording, Transcribing, Inserted and errors. Errors never
 * show as toasts; the row clears when the next dictation starts.
 */
export function DictationNotices({ dictationKey }: { dictationKey: string }) {
  const snapshot = useSnapshot();
  const mine = snapshot.key === dictationKey;
  const notice = mine ? snapshot.notice : null;
  const partial = mine && snapshot.activity === 'recording' ? snapshot.partial : '';
  const waiting = mine && snapshot.waiting;
  return (
    <>
      <span
        className="sr-only"
        role={mine && snapshot.announcement ? 'status' : undefined}
        aria-live="polite"
      >
        {mine ? snapshot.announcement : ''}
      </span>
      {(notice || partial || waiting) && (
        <div className="br-dictation-notices" data-testid="dictation-notices">
          {waiting && (
            <div className="br-composer-notice" data-testid="dictation-wait">
              <span className="text-supporting">{dictationMessage('MSG-WAIT')}</span>
            </div>
          )}
          {partial && (
            <p className="br-dictation-partial text-supporting" data-testid="dictation-partial">
              <span className="sr-only">{DICTATION_LABELS.partialLabel}: </span>
              {partial}
            </p>
          )}
          {notice && (
            <div
              className="br-composer-notice"
              data-testid="dictation-notice"
              data-failure={notice.failure ? 'true' : undefined}
            >
              <span className="text-supporting">{notice.text}</span>
              {notice.action === 'retry' && (
                <Button
                  type="button"
                  variant="link"
                  size="xs"
                  className="h-auto px-0 text-supporting"
                  onClick={() => void retryDictation(dictationKey)}
                >
                  {DICTATION_ACTIONS.retry}
                </Button>
              )}
              {notice.action === 'open-settings' && (
                <Button
                  type="button"
                  variant="link"
                  size="xs"
                  className="h-auto px-0 text-supporting"
                  onClick={() => void window.electron?.dictationOpenMicSettings?.()}
                >
                  {DICTATION_ACTIONS.openSettings}
                </Button>
              )}
              {snapshot.activity !== 'transcribing' && snapshot.activity !== 'recording' && (
                <Button
                  type="button"
                  variant="link"
                  size="xs"
                  className="h-auto px-0 text-supporting"
                  onClick={() => dismissDictationNotice(dictationKey)}
                >
                  {DICTATION_ACTIONS.dismiss}
                </Button>
              )}
            </div>
          )}
        </div>
      )}
    </>
  );
}
