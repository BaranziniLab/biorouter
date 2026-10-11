import { useEffect, useRef, type RefObject } from 'react';

/**
 * Where a dialog puts focus when its controls load AFTER it mounts (the Configure dialog's key
 * fields, the Lead/worker dialog's models). Radix picks "the first tabbable control" at mount,
 * and at that moment the body is a placeholder, so the pick was the footer's first button: Remove
 * on a configured provider, or the × in the Lead/worker dialog. A keyboard user opened a form on a
 * destructive button, or on its own close control, and the ring said so.
 *
 * Two halves, used together:
 *
 * - `restOnDialog` is the dialog's `onOpenAutoFocus`. Focus rests on the dialog box, which draws
 *   no ring (`.biorouter-modal-surface:focus`) and is where a screen reader reads the title. The
 *   shared dialog records the opener before it calls this, so closing still returns focus to it.
 * - `useHandFocusOnReady` takes focus off the box and onto the first control once the body is
 *   there, but only while the box still holds it: a person who has already moved on (Tab to
 *   Cancel, a click) is left where they went.
 */
export function restOnDialog(event: Event): void {
  const dialog = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  if (!dialog) return;
  event.preventDefault();
  dialog.focus({ preventScroll: true });
}

/**
 * Returns a ref for a node inside the dialog's body. When `ready` turns true, and the dialog box
 * is still the focused element, the first control in that node matching `selector` takes focus.
 * A selector that matches nothing (every control disabled, a host-managed form) leaves focus on
 * the box, and the next Tab reaches the first control as usual.
 */
export function useHandFocusOnReady<T extends HTMLElement>(
  ready: boolean,
  selector: string
): RefObject<T | null> {
  const bodyRef = useRef<T>(null);
  useEffect(() => {
    if (!ready) return;
    const body = bodyRef.current;
    const dialog = body?.closest('[role="dialog"]');
    if (!body || !dialog || document.activeElement !== dialog) return;
    body.querySelector<HTMLElement>(selector)?.focus({ preventScroll: true });
  }, [ready, selector]);
  return bodyRef;
}
