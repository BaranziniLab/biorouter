import { useCallback, useState } from 'react';

/**
 * T3-SH-12 — give the keyboard focus back to the control that opened a dialog.
 *
 * ⚠ Radix's modal dialog, when it closes, focuses its own `Dialog.Trigger` and
 * nothing else: it cancels the focus scope's usual return to whatever was
 * focused before it opened. A dialog opened from state, with no Trigger (every
 * Settings dialog that is mounted by a button's `onClick`), therefore left the
 * focus on `<body>` after Escape, and a keyboard user had to start again from
 * the top of the page. Crew's dialogs return focus themselves for the same
 * reason (`ModalShellDefaults.onCloseAutoFocus`).
 *
 * Call it in the dialog's component; it remembers the element focused when the
 * component first rendered, which is before Radix moves the focus inside. Pass
 * the handler as the content's `onCloseAutoFocus`. An opener that is gone by
 * the time the dialog closes (a menu item, a row that re-rendered) is left
 * alone, and Radix does what it would have done.
 */
export function useReturnFocusToOpener(): (event: Event) => void {
  const [opener] = useState<HTMLElement | null>(() => {
    if (typeof document === 'undefined') return null;
    const active = document.activeElement;
    return active instanceof HTMLElement && active !== document.body ? active : null;
  });
  return useCallback(
    (event: Event) => {
      if (!opener || !opener.isConnected) return;
      event.preventDefault();
      opener.focus();
    },
    [opener]
  );
}
