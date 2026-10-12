import { useElapsedMs } from '../hooks/useElapsedMs';
import { formatElapsed } from '../utils/formatElapsed';
import type { TrailingActivity } from '../utils/trailingActivity';
import { cn } from '../utils';
import { Spinner } from './ui/spinner';
import { MessageSquare } from './icons/app-icons';
import { TranscriptRow } from './TranscriptRow';

/** Below this the number flickers in and out on every fast tool boundary. */
export const ELAPSED_REVEAL_MS = 2000;

interface TurnActivityIndicatorProps {
  activity: TrailingActivity;
  className?: string;
  /**
   * Accepted for the callers that still pass it; nothing reads it. It steered a
   * 45 second "you can stop the turn from the composer" nudge, which is gone:
   * Stop is on screen, and the line's own clock says how long it has been.
   */
  canStopHere?: boolean;
}

/** The working glyph: the one spinner, at the row's 16px. */
function WorkingGlyph({ className }: { className?: string }) {
  return <Spinner size={16} className={className} />;
}

/**
 * The trailing activity line: `[spinner] Thinking · 12s` (spec 3.6).
 *
 * It is the transcript's one row (WS-TOOLS' `TranscriptRow`), static and
 * muted, and it breathes with the same text pulse as a running tool row, on the
 * same `--dur-slow × 4` period. The ring and glow it used to draw were a third
 * "working" loop beside the composer's edge and the tool pulse; they are gone.
 *
 * It lives INSIDE the transcript, under the last tool row, because that is
 * where the user is looking while a tool result sits on screen and the next
 * provider round-trip is in flight. Every input is derived
 * (`utils/trailingActivity.ts`) and its clock origin comes from the stream
 * store, never from mount time, so it renders the same however often the list
 * re-reconciles.
 *
 * Reduced motion: the spinner and the pulse both stand still (their own rests in
 * `main.css` and `transcript-row.css`).
 */
export default function TurnActivityIndicator({ activity, className }: TurnActivityIndicatorProps) {
  const elapsedMs = useElapsedMs(activity.since);
  const showElapsed = elapsedMs !== null && elapsedMs >= ELAPSED_REVEAL_MS;
  const elapsedLabel = showElapsed ? formatElapsed(elapsedMs) : null;
  // Screen readers get the time in 15s buckets, not the 1 Hz tick. Below the
  // first full bucket no duration is announced at all: "about 0 seconds" is
  // noise, and rounding a 3s wait up to "about 15 seconds" would be a lie.
  const announcedSeconds =
    elapsedMs !== null && elapsedMs >= 15000 ? Math.floor(elapsedMs / 15000) * 15 : null;
  // D3: nothing is working while a steer waits behind a card, so nothing
  // moves: the pulse and the spinner are claims of activity.
  const waiting = activity.phase === 'steerQueued';

  return (
    <div
      className={cn('w-full', className)}
      data-testid="turn-activity-indicator"
      data-phase={activity.phase}
    >
      <div role="status" aria-live="polite" aria-atomic="true">
        <TranscriptRow
          icon={waiting ? MessageSquare : WorkingGlyph}
          glyph={waiting ? 'waiting' : 'working'}
          running={!waiting}
          label={
            <span aria-hidden="true">
              {activity.label}
              {/* BR-61: the user's own words, so the press is visibly
                  acknowledged with the exact text that left the composer. */}
              {activity.steerText ? (
                <span data-testid="turn-activity-steer-text" className="text-text-default">
                  {' '}
                  “{activity.steerText}”
                </span>
              ) : null}
            </span>
          }
          meta={
            elapsedLabel ? (
              <span aria-hidden="true" data-testid="turn-activity-elapsed" className="tabular-nums">
                · {elapsedLabel}
              </span>
            ) : null
          }
        />
        {/*
          The visible line ticks once a second and is aria-hidden: inside a live
          region that would be about 60 announcements a minute. This sr-only
          sibling carries the announcement instead, coarsened to 15s buckets.
        */}
        <span className="sr-only">
          {activity.label}
          {activity.steerText ? `: ${activity.steerText}` : ''}
          {announcedSeconds !== null ? `, about ${announcedSeconds} seconds elapsed` : ''}
        </span>
      </div>
    </div>
  );
}
