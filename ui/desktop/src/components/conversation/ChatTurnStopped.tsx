import Stop from '../ui/Stop';

/**
 * F5 — what a Stop the daemon CONFIRMED did, said once and quietly.
 *
 * The success half of M2's notice (`ChatTurnError`'s "Stop not confirmed"). A
 * failed Stop gets a card because the user has something to do about it: the
 * turn may still be running. A confirmed one gets a line because nothing is left
 * to do. The turn is over, Send is back, and the only thing missing was the
 * words. So it carries no surface, no status hue and no action —
 * `text-supporting` in `--text-muted`, the settings vocabulary's status line
 * (rule 6) — in the slot the failed Stop's card would take.
 *
 * It borrows the trailing activity line's geometry on purpose. That line
 * (`TurnActivityIndicator`) is what the user was watching when they pressed
 * Stop, and this one lands where it was, at its height, with the Stop button's
 * own glyph where the working pulse had been: the working line's last state
 * rather than a new element.
 *
 * It is drawn from two places, and they are the same fact:
 *
 * - **the transcript**, for the notice the daemon now STORES when a Stop ends a
 *   turn (item 7, `turnStoppedNotice.ts`). That row is what a reload, another
 *   window and History show, and in the window that pressed Stop the cancel
 *   response puts it into the transcript directly. `inTranscript` drops the top
 *   margin, which the transcript's row wrapper already provides.
 * - **BaseChat's tail slot**, from the store's transient
 *   `ChatStreamSnapshot.stopConfirmed` — now only the fallback for a daemon that
 *   returned no stored record, and never beside a transcript that already ends
 *   on the stored line. It is retracted after `STOP_CONFIRMED_NOTICE_MS` or by
 *   the next turn.
 *
 * Reduced motion is handled by the global reset in `styles/main.css`.
 */
export function ChatTurnStopped({ inTranscript = false }: { inTranscript?: boolean }) {
  return (
    <div
      data-testid="chat-turn-stopped"
      className={`${inTranscript ? '' : 'mt-4 '}w-full animate-fade-slide-up`}
    >
      <div
        role="status"
        className="inline-flex items-center gap-2 px-1 py-1 text-supporting text-text-muted"
      >
        <span aria-hidden="true" className="flex h-4 w-4 flex-shrink-0 items-center justify-center">
          <Stop size={12} />
        </span>
        Stopped.
      </div>
    </div>
  );
}
