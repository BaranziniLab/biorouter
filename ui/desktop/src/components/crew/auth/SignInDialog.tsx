import { ModalShell } from '../../ModalShell';
import CrewAuthentication from '../CrewAuthentication';
import { connectionServer } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { signInCopy } from './copy';

/**
 * Sign in to {host}: the SSH password or verification-code terminal, as a dialog that must be
 * answered (`purpose="required"`). There is no ×, and neither Escape nor a click outside closes it,
 * so a stray key never orphans an SSH session; the terminal's own Close is the one way out, and it
 * ends the session. Exit 0 closes the dialog and refreshes without a second connect.
 *
 * It opens by itself when a connect the person started finds the server wants a password or a
 * code, and from the workspace menu, the status word and the sign-in screen.
 */
export function SignInDialog() {
  const { signIn, connection, connectionId, onSignedIn, closeSignIn } = useCrew();
  const host = connectionServer(connection) || connection?.name || '';
  const open = signIn.open && Boolean(connectionId);
  return (
    <ModalShell
      open={open}
      // Required: the dialog never dismisses itself. Close lives in the terminal body.
      onOpenChange={() => {}}
      size="lg"
      purpose="required"
      title={signInCopy.title(host)}
      subtitle={signInCopy.lead}
    >
      {open ? (
        <CrewAuthentication
          key={connectionId}
          connectionId={connectionId}
          onConnected={onSignedIn}
          onClose={closeSignIn}
        />
      ) : null}
    </ModalShell>
  );
}
