import { useLayoutEffect, useRef } from 'react';
import type { ChatTurnErrorData } from '../../types/turnError';
import type { TranscriptFollow } from './useTranscriptFollow';

/**
 * How long after the transcript is first drawn a row still counts as part of
 * the open. Under a reaction time: nobody has read a line or aimed at a card in
 * the first quarter second, and a restore failure on an ordinary chat lands a
 * few frames after its transcript (the agent load's resume is a second round
 * trip, `chatStreamStore.ensureAgentLoaded`).
 */
export const LATE_ROW_GRACE_MS = 250;

/**
 * Whether the turn error row that has just appeared is held below the fold
 * instead of followed (vision V2-R9-2), from what is known at that commit:
 *
 * - `openOutcome`: the row is the chat's open's own outcome: no turn has run
 *   since the chat was opened and no earlier error row was drawn. A turn's
 *   failure, a Retry's, a Stop's are the person's (or a live turn's) and the
 *   reader follows them to the end as before.
 * - `scope`: only a `'session'` error, the model and extensions the agent load
 *   could not prepare (`initialization_error`, `agent_load_failed`).
 * - `shownAt` / `now`: the transcript has been on screen for at least
 *   `LATE_ROW_GRACE_MS`. Before that (or with the row in the transcript's first
 *   frame, `shownAt` null) it joins the open and is followed into view.
 */
export function lateRowHolds({
  openOutcome,
  scope,
  shownAt,
  now,
}: {
  openOutcome: boolean;
  scope: ChatTurnErrorData['scope'];
  shownAt: number | null;
  now: number;
}): boolean {
  if (!openOutcome || scope !== 'session' || shownAt === null) return false;
  return now - shownAt >= LATE_ROW_GRACE_MS;
}

interface LateRowHoldOptions {
  sessionId: string;
  /** The transcript is mounted and draws messages (not the empty state, not a load error). */
  transcriptShown: boolean;
  /** The turn error BaseChat draws at the end of the transcript, or null when it draws none. */
  row: ChatTurnErrorData | null;
  /** A turn is running in this chat (`isRunningState(chatState)`), the person's or a live one. */
  turnRunning: boolean;
  follow: Pick<TranscriptFollow, 'holdForLateRow' | 'releaseHold'>;
}

/**
 * A chat whose model cannot be restored learns it from the agent load, the
 * second `/agent/resume` an open makes, off the paint path on purpose. On a
 * long chat that answer lands seconds after the transcript (measured on an
 * 82-figure chat: drawn at 6.0 s, the error at 12.5 s), and drawing the row at
 * the end while the reader followed it moved every card on screen 62px up in
 * one frame, under the pointer (V2-R9-2).
 *
 * The outcome cannot be known before the transcript is drawn without holding
 * every open for the agent load (seconds of extension startup on a real chat),
 * so a late one is added WITHOUT moving what is on screen: the row goes below
 * the fold and nothing follows it there (`TranscriptFollow.holdForLateRow`)
 * until the reader reaches the end, or their next send takes them there and
 * answers with the same failure in place.
 *
 * Decided in the layout phase of the commit that draws the row, so the scroll
 * area's follow, which runs in the passive effects after it, already sees the
 * hold.
 */
export function useLateRowHold({
  sessionId,
  transcriptShown,
  row,
  turnRunning,
  follow,
}: LateRowHoldOptions): void {
  const sessionRef = useRef(sessionId);
  const shownAtRef = useRef<number | null>(null);
  const openOutcomeRef = useRef(true);
  const rowRef = useRef<ChatTurnErrorData | null>(null);
  const { holdForLateRow, releaseHold } = follow;

  useLayoutEffect(() => {
    if (sessionRef.current !== sessionId) {
      sessionRef.current = sessionId;
      shownAtRef.current = null;
      openOutcomeRef.current = true;
      rowRef.current = null;
      releaseHold();
    }
    // A turn that runs ends the open: what fails after it is the turn's.
    if (turnRunning) openOutcomeRef.current = false;

    const now = performance.now();
    if (row !== rowRef.current) {
      rowRef.current = row;
      if (!row) {
        releaseHold();
      } else {
        const hold = lateRowHolds({
          openOutcome: openOutcomeRef.current,
          scope: row.scope,
          shownAt: shownAtRef.current,
          now,
        });
        openOutcomeRef.current = false;
        if (hold) holdForLateRow();
        else releaseHold();
      }
    }
    // After the decision: a row drawn in the transcript's first frame is part
    // of it, not late.
    if (transcriptShown && shownAtRef.current === null) shownAtRef.current = now;
  }, [sessionId, transcriptShown, row, turnRunning, holdForLateRow, releaseHold]);
}
