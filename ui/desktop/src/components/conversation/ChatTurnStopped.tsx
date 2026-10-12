import Stop from '../ui/Stop';
import { TranscriptRow } from '../TranscriptRow';
import { cn } from '../../utils';
import { TURN_STOPPED_NOTICE } from './turnStoppedNotice';

/** The Stop button's own square, on the row's 16px glyph box. */
function StopGlyph() {
  return <Stop size={16} />;
}

/** The words the daemon stores for a confirmed Stop, so both lines read the same. */
const STOPPED = TURN_STOPPED_NOTICE;

/**
 * F5 — what a Stop the daemon CONFIRMED did, said once and quietly.
 *
 * The success half of M2's notice (`ChatTurnError`'s "Stop not confirmed"). A
 * failed Stop gets a card because the user has something to do about it: the
 * turn may still be running. A confirmed one gets a line because nothing is left
 * to do. The turn is over, Send is back, and the only thing missing was the
 * words. So it carries no surface, no status hue and no action: the transcript's
 * one row (`TranscriptRow`), static, in `text-secondary` muted ink.
 *
 * It is the trailing activity line's own row on purpose. That line
 * (`TurnActivityIndicator`) is what the user was watching when they pressed
 * Stop, and this one lands where it was, at its height, with the Stop button's
 * own square where the working spinner had been: the working line's last state
 * rather than a new element.
 *
 * It is drawn from two places, and they are the same fact:
 *
 * - **the transcript**, for the notice the daemon now STORES when a Stop ends a
 *   turn (item 7, `turnStoppedNotice.ts`). That row is what a reload, another
 *   window and History show, and in the window that pressed Stop the cancel
 *   response puts it into the transcript directly. `inTranscript` drops the top
 *   margin and the entrance, which the transcript's row wrapper already
 *   provides.
 * - **BaseChat's tail slot**, from the store's transient
 *   `ChatStreamSnapshot.stopConfirmed` — now only the fallback for a daemon that
 *   returned no stored record, and never beside a transcript that already ends
 *   on the stored line. It is retracted after `STOP_CONFIRMED_NOTICE_MS` or by
 *   the next turn.
 *
 * Live in the tail slot it enters with the shared `.br-enter` (its own
 * reduced-motion rest in `main.css`).
 */
export function ChatTurnStopped({ inTranscript = false }: { inTranscript?: boolean }) {
  return (
    <div data-testid="chat-turn-stopped" className={cn('w-full', !inTranscript && 'mt-2 br-enter')}>
      <div role="status">
        <TranscriptRow icon={StopGlyph} glyph="stopped" label={STOPPED} />
      </div>
    </div>
  );
}
