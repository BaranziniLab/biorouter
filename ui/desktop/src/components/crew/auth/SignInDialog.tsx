import { useLayoutEffect, useRef } from 'react';
import { ModalShell } from '../../ModalShell';
import CrewAuthentication from '../CrewAuthentication';
import { connectionServer } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { openerOf, restoreFocus, SIGN_IN_FOCUS_FALLBACKS } from '../state/focusReturn';
import { signInCopy } from './copy';

/**
 * Sign in to {host}: the SSH password or verification-code terminal, as a dialog that must be
 * answered (`purpose="required"`). There is no ×, and neither Escape nor a click outside closes it,
 * so a stray key never orphans an SSH session; the terminal's own Close is the one way out, and it
 * ends the session. Exit 0 closes the dialog and refreshes without a second connect.
 *
 * It opens by itself when a connect the person started finds the server wants a password or a
 * code, and from the workspace menu, the status word and the sign-in screen.
 *
 * Focus returns to whatever had it when the dialog opened (the workspace menu's trigger for a menu
 * item), else to the workspace switcher — never to `<body>`, where a keyboard user would start
 * again from the top of the window (QA T-15). It is recorded before the dialog moves focus into
 * itself, and restored when the dialog's content has actually left, after its exit animation.
 */
export function SignInDialog() {
  const { signIn, connection, connectionId, onSignedIn, closeSignIn } = useCrew();
  const host = connectionServer(connection) || connection?.name || '';
  const open = signIn.open && Boolean(connectionId);
  const opener = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);

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
