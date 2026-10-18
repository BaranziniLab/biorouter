import { useLayoutEffect, useRef } from 'react';
import { ModalShell } from '../../ModalShell';
import CrewAuthentication, { type CrewSignInTarget } from '../CrewAuthentication';
import { serverLabel } from '../sidebar/sidebarView';
import { useCrew } from '../state/CrewControllerContext';
import { openerOf, restoreFocus, SIGN_IN_FOCUS_FALLBACKS } from '../state/focusReturn';
import { signInCopy } from './copy';

export interface SignInDialogProps {
  /**
   * What to sign in for, when the dialog is not the controller's: a setup job waiting on the
   * person (OW-9). Without it the dialog signs in to the selected connection, opened and closed by
   * the controller's `signIn`.
   */
  target?: CrewSignInTarget;
  /** With `target`: whether it is open, and the server's name for the title. */
  open?: boolean;
  host?: string;
  /** With `target`: the sign-in finished (exit 0), or the person closed it. */
  onSignedIn?: () => void;
  onClose?: () => void;
}

/**
 * Sign in to {host}: the SSH password or verification-code terminal, as a dialog that must be
 * answered (`purpose="required"`). There is no ×, and neither Escape nor a click outside closes it,
 * so a stray key never orphans an SSH session; the terminal's own Close is the one way out, and it
 * ends the session. Exit 0 closes the dialog and refreshes without a second connect.
 *
 * It opens by itself when a connect the person started finds the server wants a password or a
 * code, and from the workspace menu, the status word and the sign-in screen. A setup job that needs
 * a sign-in opens its own (`target`), from the Host dialog or the Not set up pane.
 *
 * Focus returns to whatever had it when the dialog opened (the workspace menu's trigger for a menu
 * item), else to the workspace switcher — never to `<body>`, where a keyboard user would start
 * again from the top of the window (QA T-15). It is recorded before the dialog moves focus into
 * itself, and restored when the dialog's content has actually left, after its exit animation.
 */
export function SignInDialog(props: SignInDialogProps = {}) {
  if (props.target) {
    return (
      <SignInModal
        open={props.open ?? true}
        host={props.host ?? ''}
        target={props.target}
        onSignedIn={props.onSignedIn ?? (() => {})}
        onClose={props.onClose ?? (() => {})}
      />
    );
  }
  return <ConnectionSignInDialog />;
}

/** The controller's sign-in to the selected connection. */
function ConnectionSignInDialog() {
  const { signIn, connection, connectionId, onSignedIn, closeSignIn, connect } = useCrew();
  // The server by the person's own name for it, as the main area names it (DW-03).
  const host = serverLabel(connection) || connection?.name || '';
  const open = signIn.open && Boolean(connectionId);
  return (
    <SignInModal
      open={open}
      host={host}
      target={{ kind: 'connection', id: connectionId }}
      onSignedIn={onSignedIn}
      onClose={closeSignIn}
      // Signed in, but Crew isn't installed for the account (OW-9): the daemon kept the session,
      // so connecting again over it lands on the Not set up pane, whose setup uses that session.
      onBridgeMissing={() => {
        closeSignIn();
        void connect({ userInitiated: true });
      }}
    />
  );
}

function SignInModal({
  open,
  host,
  target,
  onSignedIn,
  onClose,
  onBridgeMissing,
}: {
  open: boolean;
  host: string;
  target: CrewSignInTarget;
  onSignedIn: () => void;
  onClose: () => void;
  onBridgeMissing?: () => void;
}) {
  const opener = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);
  const targetKey = target.kind === 'connection' ? target.id : `setup:${target.jobId}`;

  // A layout effect runs before Radix's focus scope (a passive effect) moves focus into the dialog.
  useLayoutEffect(() => {
    if (open && !wasOpen.current) opener.current = openerOf(document.activeElement);
    wasOpen.current = open;
  }, [open]);

  return (
    <ModalShell
      open={open}
      // Required: the dialog never dismisses itself. Close lives in the terminal body.
      onOpenChange={() => {}}
      size="lg"
      purpose="required"
      title={signInCopy.title(host)}
      subtitle={signInCopy.lead}
      onCloseAutoFocus={(event) => {
        event.preventDefault();
        const target = opener.current;
        opener.current = null;
        restoreFocus(target, SIGN_IN_FOCUS_FALLBACKS);
      }}
    >
      {open ? (
        target.kind === 'connection' ? (
          <CrewAuthentication
            key={targetKey}
            connectionId={target.id}
            onConnected={onSignedIn}
            onClose={onClose}
            onBridgeMissing={onBridgeMissing}
          />
        ) : (
          <CrewAuthentication
            key={targetKey}
            target={target}
            onConnected={onSignedIn}
            onClose={onClose}
          />
        )
      ) : null}
    </ModalShell>
  );
}
