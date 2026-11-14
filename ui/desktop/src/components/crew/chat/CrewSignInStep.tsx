import { useEffect, useState } from 'react';
import type { CrewStepRequest } from '../../../api';
import { isBrowserSurface } from '../../../utils/surface';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import { SignInDialog } from '../auth/SignInDialog';
import type { CrewSignInTarget } from '../CrewAuthentication';
import { sanitizeDisplayText } from '../identity/displayText';
import { crewSignInStepCopy } from './CrewSignInStep.copy';
import { CrewStepFrame, dismissStep } from './CrewStepFrame';
import { useSignInWindowName } from './signInWindowName';

type SignIn = Extract<CrewStepRequest, { kind: 'signIn' }>;

/** Where the card stands, as this window saw it. */
type Seen = 'waiting' | 'dismissing' | 'signedIn' | 'dismissed' | 'gone';

/** The Sign in window's target for the daemon's own target on the card. */
function windowTarget(target: SignIn['target']): CrewSignInTarget {
  return target.kind === 'setup'
    ? { kind: 'setup', jobId: target.jobId }
    : { kind: 'connection', id: target.connectionId };
}

/**
 * Whether this window can open the Sign in window for `target`: the desktop app's main process
 * runs the ssh terminal, so a browser (`biorouter serve`) or an app without the bridge cannot.
 */
function canOpenSignIn(target: CrewSignInTarget): boolean {
  if (isBrowserSurface()) return false;
  const electron = typeof window === 'undefined' ? undefined : window.electron;
  const create =
    target.kind === 'setup'
      ? electron?.createCrewSetupAuthentication
      : electron?.createCrewAuthentication;
  return typeof create === 'function';
}

/**
 * A chat's sign-in card (the owner's D1 ruling on AO-12): sign in to the server the daemon named,
 * as the person. **Sign in** opens the existing Sign in window (`SignInDialog`, an ssh terminal
 * in a modal, never an inline one: an xterm that unmounts comes back blank in the middle of a
 * prompt). What the person types goes from that window to ssh through the main process with
 * their proof, never through this card, the chat or the model; the card holds no input.
 *
 * The card is never answered as done. The chat's call watches the daemon, which takes the card
 * down once the job moves on or the sign-in is adopted; **Dismiss** only ends the chat's wait
 * with nothing signed in. Where no Sign in window can open, the card says where to sign in.
 *
 * The Sign in window's title names what the target's ID is by the daemon's own read
 * (`signInWindowName.ts`), not by the card's words, since the window connects to that ID.
 */
export default function CrewSignInStep({
  sessionId,
  id,
  step,
  onHolding,
}: {
  sessionId: string;
  id: string;
  step: SignIn;
  /** Told whether the Sign in window is open, so the card stays while the person is in it. */
  onHolding?: (holding: boolean) => void;
}) {
  const login = sanitizeDisplayText(step.login);
  const server = sanitizeDisplayText(step.server);
  const target = windowTarget(step.target);
  const [open, setOpen] = useState(false);
  const [seen, setSeen] = useState<Seen>('waiting');
  const signInHere = canOpenSignIn(target);
  const windowName = useSignInWindowName(target, signInHere);
  useEffect(() => {
    onHolding?.(open);
  }, [open, onHolding]);

  const dismiss = async () => {
    setSeen('dismissing');
    let dismissed = false;
    try {
      dismissed = await dismissStep(sessionId, id);
    } catch {
      dismissed = false;
    }
    setSeen(dismissed ? 'dismissed' : 'gone');
  };

  const said =
    seen === 'signedIn'
      ? crewSignInStepCopy.signedIn
      : seen === 'dismissed'
        ? crewSignInStepCopy.dismissed
        : seen === 'gone'
          ? crewSignInStepCopy.gone
          : null;
  const acting = seen === 'waiting' || seen === 'dismissing';

  // Where the typing goes stays in view; which server and where to type wait behind the title's
  // info icon. Where no Sign in window opens, the warning stays and the way to sign in folds
  // (owner, 2026-11-13).
  return (
    <CrewStepFrame
      kind="signIn"
      title={crewSignInStepCopy.title(login)}
      titleHelp={
        signInHere
          ? { label: crewSignInStepCopy.aboutLabel, help: crewSignInStepCopy.about(login, server) }
          : undefined
      }
    >
      <p className="text-supporting text-text-muted">
        {signInHere ? crewSignInStepCopy.lead : crewSignInStepCopy.fallback}
      </p>
      {signInHere ? null : (
        <Disclosure label={crewSignInStepCopy.fallbackHowLabel}>
          <p className="text-supporting text-text-muted">{crewSignInStepCopy.fallbackHow}</p>
        </Disclosure>
      )}
      {acting ? (
        <div className="flex flex-wrap items-center gap-2">
          {signInHere ? (
            <Button
              type="button"
              size="sm"
              aria-label={crewSignInStepCopy.signInLabel(login, server)}
              disabled={seen === 'dismissing'}
              onClick={() => setOpen(true)}
            >
              {crewSignInStepCopy.signIn}
            </Button>
          ) : null}
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={seen === 'dismissing'}
            onClick={() => void dismiss()}
          >
            {crewSignInStepCopy.dismiss}
          </Button>
        </div>
      ) : null}
      {said ? (
        <p className="text-supporting text-text-muted" role="status">
          {said}
        </p>
      ) : null}
      {signInHere ? (
        <SignInDialog
          target={target}
          open={open}
          host={windowName ?? crewSignInStepCopy.windowFallback}
          onSignedIn={() => {
            setOpen(false);
            setSeen('signedIn');
          }}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </CrewStepFrame>
  );
}
