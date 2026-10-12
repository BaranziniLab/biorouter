import { useCallback, useRef } from 'react';
import { lastInputWasKey } from '../../ui/inputTrail';

/**
 * Gives keyboard focus back to the knowledge-base selector when the manager, or the chooser it
 * starts, is closed (V3-R7-3).
 *
 * Both are opened by a row in the picker's popover, and the popover is gone by the time the
 * dialog closes. The dialog primitive returns focus to the control that opened it
 * (`useReturnFocusToOpener`), but it reads that control when the dialog MOUNTS, and the row that
 * was pressed, or the search field that held focus, is still on screen then, playing the popover's
 * close motion. It is detached when the dialog closes, so nothing was given back and Escape left
 * `document.activeElement` on `<body>`: a keyboard user began again at the top of the page.
 * jsdom has no motion, so the popover is already gone and focus is already back on the trigger
 * when the dialog opens; only a real browser shows it (`KnowledgeView.focusReturn.browser.test`).
 *
 * The caller names the opener itself instead: the selector's trigger, held in `triggerRef`. The
 * rule is the dialog primitive's own, which asks how the dialog was CLOSED and not how it was
 * opened (`useReturnFocusToOpener`, V1-R8-1), so a pointer user is never shown a ring they did not
 * ask for and a keyboard user is never dropped onto `<body>`:
 *
 * - It gives focus back when the dialog is closed with the KEYBOARD (Escape, or Enter on a
 *   button), however it was opened. A trigger clicked open, "Manage bases..." clicked and Escape
 *   used to leave focus on `<body>` and the next Tab began at the top of the window (V3-R9-1). A
 *   close with the pointer gives nothing back: a script focus after a mouse click can wear a
 *   keyboard ring, and the person is already where they pointed.
 * - It needs the trigger to be on the page. The empty states' buttons open the manager too, and
 *   the trigger is drawn only while a base is primary, so those are connected controls the dialog
 *   primitive returns to by itself and this stands aside. (If the person makes a base primary in
 *   the manager, the button is gone and the trigger has appeared: that is the one to go back to.)
 * - Only a real close counts. The manager is unmounted while the chooser takes its place and
 *   comes back under it, and that unmount also reports a close; `closed()` tells the two apart,
 *   so the swap neither spends the hand-back nor moves focus off the chooser.
 * - A focus that already sits somewhere else (outside the closing dialog and not on `<body>`) is
 *   where the person went, and is left alone.
 */
export function useSelectorFocusReturn() {
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  // An open that has not been answered yet: the one close after it is the one this decides.
  const pending = useRef(false);
  // Between `opened` and `closed`: the manager is up, or the chooser is standing in for it.
  const isOpen = useRef(false);

  /** The manager (or the chooser) is opening, by whatever the last input was. */
  const opened = useCallback(() => {
    isOpen.current = true;
    pending.current = true;
  }, []);

  /** The dialog was dismissed for good (not swapped for the chooser). */
  const closed = useCallback(() => {
    isOpen.current = false;
  }, []);

  /** Radix's close-focus hook for the manager; it runs before the dialog primitive's own return. */
  const onCloseAutoFocus = useCallback((event: Event) => {
    if (isOpen.current || !pending.current) return;
    pending.current = false;
    const active = document.activeElement;
    const content = event.target instanceof Node ? event.target : null;
    if (active && active !== document.body && content && !content.contains(active)) return;
    const target = triggerRef.current;
    if (!lastInputWasKey() || !target || !target.isConnected) return;
    event.preventDefault();
    target.focus();
  }, []);

  return { triggerRef, opened, closed, onCloseAutoFocus };
}
