import { useEffect, useRef, type RefObject } from 'react';
import { lastInputWasKey } from '../../ui/inputTrail';

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
 *
 * **A pointer open hands focus only to a text field** (vision round 4, V4-R4-1). A focus a script
 * moves after a mouse click can match `:focus-visible` (Chromium decides from the menu item the
 * pointer just picked), so the Lead/worker switch, handed focus once its models loaded, wore the
 * inset grey ring after "Lead and worker..." was clicked. A text field is always ringed and wants
 * the cursor, so it still takes focus. Any other first control stays where the box left it, which
 * draws no ring, and the next Tab reaches it. A keyboard open hands focus to whatever comes first,
 * as it always did, because there the ring is the point. The modality is read where
 * `ui/dialog.tsx` reads it for its own quiet open: `ui/inputTrail`'s tracker.
 */

/** Whether the input that opened each dialog was a pointer press (not a key). */
const openedWithPointer = new WeakMap<Element, boolean>();

const NOT_TEXT_INPUT_TYPES = new Set([
  'hidden',
  'checkbox',
  'radio',
  'range',
  'button',
  'submit',
  'reset',
  'file',
  'image',
]);

/** A control the browser draws its own focus for, in every input modality. */
function isTextEntry(element: HTMLElement): boolean {
  if (element instanceof HTMLTextAreaElement || element.isContentEditable) return true;
  return element instanceof HTMLInputElement && !NOT_TEXT_INPUT_TYPES.has(element.type);
}

export function restOnDialog(event: Event): void {
  const dialog = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  if (!dialog) return;
  event.preventDefault();
  openedWithPointer.set(dialog, !lastInputWasKey());
  dialog.focus({ preventScroll: true });
}

/**
 * Returns a ref for a node inside the dialog's body. When `ready` turns true, and the dialog box
 * is still the focused element, the first control in that node matching `selector` takes focus
 * (after a pointer open, only when that control is a text field: see above). A selector that
 * matches nothing (every control disabled, a host-managed form) leaves focus on the box, and the
 * next Tab reaches the first control as usual.
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
    const first = body.querySelector<HTMLElement>(selector);
    if (!first) return;
    if (openedWithPointer.get(dialog) && !isTextEntry(first)) return;
    first.focus({ preventScroll: true });
  }, [ready, selector]);
  return bodyRef;
}
