import type { ReactNode } from 'react';
import { dismissCrewStep } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import { Lock } from '../../icons/app-icons';
import { crewStepCopy } from './CrewStepCard.copy';

/**
 * The frame every Crew step card shares (the owner's D1 ruling on AO-12): a
 * title, the line that says the card acts as the person, then the step's own
 * body. A card is the ask, never the answer, so the frame holds no input and
 * a step body must never add one: what the person types goes to the window the
 * step opens (the Sign in window, the OS-native vault prompt), never here.
 */
export function CrewStepFrame({
  kind,
  title,
  children,
}: {
  /** The step's kind, stamped as `data-crew-step` for tests and probes. */
  kind: string;
  /** Fixed text plus the daemon's sanitized values. */
  title: string;
  children?: ReactNode;
}) {
  return (
    <div
      className="biorouter-message-content br-action-card br-fill-surface br-enter text-body"
      data-testid="crew-step-card"
      data-crew-step={kind}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <div className="text-label text-text-default">{title}</div>
        <div className="flex items-center gap-1.5 text-supporting text-text-muted">
          <Lock aria-hidden="true" className="size-3.5 shrink-0" />
          <span>{crewStepCopy.actsAsYou}</span>
        </div>
      </div>
      {children}
    </div>
  );
}

/**
 * What a Crew step draws where nothing waits on it: one fixed line, with no button and nothing
 * the card carried (`crewStepCopy.notWaiting`).
 */
export function CrewStepEnded({ kind }: { kind: string }) {
  return (
    <p
      className="biorouter-message-content text-supporting text-text-muted"
      data-testid="crew-step-ended"
      data-crew-step={kind}
    >
      {crewStepCopy.notWaiting}
    </p>
  );
}

/**
 * Dismiss a Crew step card (`POST /action-required/crew-step`). The parked
 * call is released as not done. A dismissal grants nothing, but it names a
 * chat, so the daemon asks whether this caller reaches it (`session_reach`):
 * the request carries `userActionHeaders()`, as every call naming a chat does,
 * or a private chat's card could not be dismissed. Answers whether the daemon
 * still had the card.
 */
export async function dismissStep(sessionId: string, id: string): Promise<boolean> {
  const response = await dismissCrewStep({
    headers: await userActionHeaders(),
    body: { id, session_id: sessionId, outcome: 'dismiss' },
  });
  return (response.data as { status?: string } | undefined)?.status === 'dismissed';
}
