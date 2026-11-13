import { useState } from 'react';
import type { ActionRequired, CrewStepRequest } from '../../../api';
import CrewHostKeyStep from './CrewHostKeyStep';
import CrewSignInStep from './CrewSignInStep';
import CrewVaultStep from './CrewVaultStep';
import { crewStepCopy } from './CrewStepCard.copy';
import { CrewStepEnded, CrewStepFrame } from './CrewStepFrame';
import { useCardEnding, useStreamedCrewStep } from './streamedCrewSteps';

/** The step kinds this app draws; anything else is `unknown`. */
const KINDS = new Set(['signIn', 'vault', 'hostKey']);

/**
 * A Crew step card in a chat (the owner's D1 ruling on AO-12): a step only the
 * person can take, which the chat opened for them. Signing in, Crew's vault
 * and a server's host key each draw their own card; a step this app does not
 * know (a newer daemon's) draws fixed words that send the person to Crew.
 *
 * ⚠ The card is the ask, never the answer. It holds no input, and nothing on
 * it ever reaches the conversation: the step happens in the person's own
 * window, the call that parked the card reads the daemon to learn whether it
 * was done, and the only thing a card can post back is a dismissal.
 *
 * ⚠ **A card draws its buttons only while the daemon waits on it** (WI-17): this
 * window received it on the chat's live stream (`streamedCrewSteps.ts`), the
 * daemon has not said it ended, and the chat's turn still runs (`turnActive`, the
 * floor for a stream that can carry no end, since no card outlives its turn). The
 * end comes as the daemon's `ActionRequiredEnded` frame, so a card withdrawn,
 * dismissed somewhere else or out of time says so at once. The daemon never
 * stores a step card that waits, so one read back from a transcript (History, an
 * imported session file, or the rows of a chat whose turn ended) draws one fixed
 * line instead, naming nothing the card carried: there its words are whatever
 * the file says. (A shared session's view draws no action card at all.) A card
 * whose window the person has open (the Sign in window, the host-key terminal,
 * the vault prompt) stays until they close it, so an end never pulls a window
 * out from under them.
 *
 * A changed host key's card is a notice, not a step: it waits for nothing, and
 * the daemon keeps it (user-only) so a reload still shows it. Streamed, it draws
 * its details whatever happens next; read back, it draws one fixed line.
 */
export default function CrewStepCard({
  sessionId,
  actionRequiredContent,
  turnActive,
}: {
  sessionId: string;
  actionRequiredContent: ActionRequired & { type: 'actionRequired' };
  /** Whether the chat's turn is still running: false on every read-only surface. */
  turnActive: boolean;
}) {
  const data = actionRequiredContent.data;
  const id = data.actionType === 'crewStep' ? data.id : '';
  const streamed = useStreamedCrewStep(sessionId, id);
  const ending = useCardEnding(sessionId, id);
  const [holding, setHolding] = useState(false);
  if (data.actionType !== 'crewStep') return null;
  // Typed as the daemon's union, read as unknown: a newer daemon may send a step
  // this app has never heard of, and that must fall through to the fixed words.
  const step = data.step as CrewStepRequest | undefined;
  const kind = step && KINDS.has(step.kind) ? step.kind : 'unknown';
  if (step?.kind === 'hostKey' && step.changed === true) {
    return streamed ? (
      <CrewHostKeyStep sessionId={sessionId} id={id} step={step} onHolding={setHolding} />
    ) : (
      <CrewStepEnded kind={kind} line={crewStepCopy.changedKeyNotice} />
    );
  }
  const waits = streamed && ending === undefined && turnActive;
  if (!(waits || holding)) {
    // How it ended, only for a card this window saw the daemon end; a stored one names nothing.
    return <CrewStepEnded kind={kind} ending={streamed ? ending : undefined} />;
  }
  switch (step?.kind) {
    case 'signIn':
      return <CrewSignInStep sessionId={sessionId} id={id} step={step} onHolding={setHolding} />;
    case 'vault':
      return <CrewVaultStep sessionId={sessionId} id={id} step={step} onHolding={setHolding} />;
    case 'hostKey':
      return <CrewHostKeyStep sessionId={sessionId} id={id} step={step} onHolding={setHolding} />;
    default:
      return <CrewStepFrame kind="unknown" title={crewStepCopy.unknown} />;
  }
}
