import type { ReactNode } from 'react';
import { dismissCrewStep } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import { Lock } from '../../icons/app-icons';
import { InfoTip } from '../../ui/info-tip';
import { crewStepCopy } from './CrewStepCard.copy';
import type { CardEnding } from './streamedCrewSteps';

/** Words folded behind an info icon: what the icon is about, and the help itself. */
export interface StepHelp {
  /** The icon's accessible name reads "About {label}". */
  label: string;
  /** At most two sentences (design.md 9.16). */
  help: string;
}

/** An InfoTip after a line of text, with the space a sentence would leave before it. */
function HelpAfter({ help }: { help?: StepHelp }) {
  if (!help) return null;
  return (
    <>
      {' '}
      <InfoTip label={help.label} help={help.help} />
    </>
  );
}

/**
 * The frame every Crew step card shares (the owner's D1 ruling on AO-12): a
 * title, the line that says the card acts as the person, then the step's own
 * body. A card is the ask, never the answer, so the frame holds no input and
 * a step body must never add one: what the person types goes to the window the
 * step opens (the Sign in window, the OS-native vault prompt), never here.
 *
 * A step keeps one line of its own in view, the one that says where the typing goes, and what
 * explains the step waits behind the info icon beside the title (`titleHelp`; owner,
 * 2026-11-13, design.md 9.16).
 */
export function CrewStepFrame({
  kind,
  title,
  titleHelp,
  children,
}: {
  /** The step's kind, stamped as `data-crew-step` for tests and probes. */
  kind: string;
  /** Fixed text plus the daemon's sanitized values. */
  title: string;
  /** What the step is, folded behind an info icon beside the title. */
  titleHelp?: StepHelp;
  children?: ReactNode;
}) {
  return (
    <div
      className="biorouter-message-content br-action-card br-fill-surface br-enter text-body"
      data-testid="crew-step-card"
      data-crew-step={kind}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <div className="text-label text-text-default">
          <span>{title}</span>
          <HelpAfter help={titleHelp} />
        </div>
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
 * the card carried. For a card this window saw the daemon end (WI-17), the line says how it
 * ended (`crewStepCopy.ended`); for one read back from a transcript, only that nothing waits
 * (`crewStepCopy.notWaiting`), or the fixed `line` its caller names, with any more it has to say
 * behind an info icon (`help`).
 */
export function CrewStepEnded({
  kind,
  ending,
  line,
  help,
}: {
  kind: string;
  /** How the daemon said it ended, when this window saw it end. */
  ending?: CardEnding;
  /** A fixed line in place of the default one. */
  line?: string;
  /** Fixed words that follow `line`, folded behind an info icon beside it. */
  help?: StepHelp;
}) {
  return (
    <p
      className="biorouter-message-content text-supporting text-text-muted"
      data-testid="crew-step-ended"
      data-crew-step={kind}
      data-ending={ending}
    >
      {line ?? (ending ? crewStepCopy.ended[ending] : crewStepCopy.notWaiting)}
      <HelpAfter help={help} />
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
